//! AVIF writer: rav1e (pure Rust, without its assembly, ADR 0021) encodes one AV1 still frame,
//! and an alpha frame when alpha is kept; avif-serialize writes the container.
//!
//! - 8-bit samples, or 16-bit samples written at 10 bits; full range, 4:4:4.
//! - YCbCr (BT.709, or BT.2020 for Rec.2020 primaries), quality 0–100 (ravif's mapping to
//!   rav1e's quantizer). Always lossy: rav1e has no lossless mode yet.
//! - Gray images are monochrome AV1.
//! - The color space is declared with H.273 code points (`colr`, and the AV1 sequence header):
//!   the spaces PNG's cICP can name, HDR (PQ, HLG) included.
//! - The whole frame is held (in rav1e's frames, filled as rows come): at most [`MAX_SIDE`]
//!   pixels per side, the limit common decoders apply. Encoding happens at the end, and can be
//!   cancelled until it starts.

use std::fs::File;
use std::io::{BufWriter, Write};

use rav1e::color::{
    ChromaSamplePosition, ChromaSampling, ColorDescription, ColorPrimaries, MatrixCoefficients,
    PixelRange, TransferCharacteristics,
};
use rav1e::config::SpeedSettings;
use rav1e::data::{FrameType, Rational};
use rav1e::prelude::SceneDetectionSpeed;
use rav1e::{Config, Context, EncoderConfig, EncoderStatus, Frame, Pixel};
use slopshop_core::color::{AlphaMode, ChannelLayout, PixelFormat, SampleType};
use slopshop_core::{CancelToken, Size};

use super::bmp::rows_out_of_order;
use super::png::cicp_code;
use super::{ExportError, ExportNotice};

/// The H.273 primaries and transfer an AVIF declares for `space`: those of PNG's cICP, but the
/// obsolete gamma 2.2 and 2.8 curves (BT.470), which AVIF tools deprecate.
pub(super) fn avif_code(space: &slopshop_core::ColorSpace) -> Option<[u8; 2]> {
    cicp_code(space).filter(|[_, transfer]| ![4, 5].contains(transfer))
}

/// Largest side: what common decoders accept for one image (larger ones need grids).
pub const MAX_SIDE: u32 = 16384;

/// rav1e's speed preset (0 slowest, 10 fastest): a balance for still images.
const SPEED: u8 = 6;

/// rav1e's quantizer for a quality of 0–100 (ravif's mapping).
fn quantizer(quality: u8) -> u8 {
    let q = f32::from(quality.clamp(1, 100)) / 100.0;
    let x = if q >= 0.82 {
        (1.0 - q) * 2.6
    } else if q > 0.25 {
        q.mul_add(-0.5, 1.0 - 0.125)
    } else {
        1.0 - q
    };
    (x * 255.0).round() as u8
}

/// The frames being filled, at 8 or 10 bits.
enum Frames {
    Eight(Planes<u8>),
    Ten(Planes<u16>),
}

/// rav1e's contexts and frames of the color image and the alpha plane.
struct Planes<T: Pixel> {
    color: (Context<T>, Frame<T>),
    alpha: Option<(Context<T>, Frame<T>)>,
}

pub(super) struct AvifWriter {
    out: BufWriter<File>,
    frames: Frames,
    size: Size,
    channels: usize,
    gray: bool,
    sixteen: bool,
    /// `Kr`, `Kb` of the YCbCr matrix.
    matrix: (f32, f32),
    code: [u8; 3],
    next_row: u32,
    cancel: CancelToken,
}

impl AvifWriter {
    pub(super) fn new(
        file: File,
        size: Size,
        target: PixelFormat,
        quality: u8,
        cancel: CancelToken,
    ) -> Result<Self, ExportError> {
        let invalid = || ExportError::InvalidSpec(format!("AVIF cannot store {target:?}"));
        if size.width > MAX_SIDE || size.height > MAX_SIDE {
            return Err(ExportError::TooLarge {
                width: size.width,
                height: size.height,
            });
        }
        let (gray, alpha) = match target.layout {
            ChannelLayout::Gray => (true, false),
            ChannelLayout::GrayAlpha => (true, true),
            ChannelLayout::Rgb => (false, false),
            ChannelLayout::Rgba => (false, true),
        };
        if alpha && target.alpha != AlphaMode::Straight {
            return Err(invalid());
        }
        let sixteen = match target.sample {
            SampleType::U8 => false,
            SampleType::U16 => true,
            SampleType::F16 | SampleType::F32 => return Err(invalid()),
        };
        let [primaries, transfer] = avif_code(&target.color_space)
            .ok_or(ExportError::UnsupportedSpace(target.color_space))?;
        let (matrix, matrix_code) = if primaries == 9 {
            ((0.2627, 0.0593), 9)
        } else {
            ((0.2126, 0.0722), 1)
        };
        let q = quantizer(quality);
        let description = ColorDescription {
            color_primaries: match primaries {
                1 => ColorPrimaries::BT709,
                9 => ColorPrimaries::BT2020,
                _ => ColorPrimaries::SMPTE432,
            },
            transfer_characteristics: match transfer {
                1 => TransferCharacteristics::BT709,
                8 => TransferCharacteristics::Linear,
                13 => TransferCharacteristics::SRGB,
                16 => TransferCharacteristics::SMPTE2084,
                _ => TransferCharacteristics::HLG,
            },
            matrix_coefficients: match (gray, matrix_code) {
                (true, _) => MatrixCoefficients::Unspecified,
                (false, 9) => MatrixCoefficients::BT2020NCL,
                (false, _) => MatrixCoefficients::BT709,
            },
        };
        let (w, h) = (size.width as usize, size.height as usize);
        let depth = if sixteen { 10 } else { 8 };
        let color_sampling = if gray {
            ChromaSampling::Cs400
        } else {
            ChromaSampling::Cs444
        };
        let encode_error = |e: rav1e::InvalidConfig| ExportError::Encode(format!("AV1: {e}"));
        let frames = if sixteen {
            Frames::Ten(Planes {
                color: context(w, h, depth, color_sampling, Some(description), q)
                    .map_err(encode_error)?,
                alpha: alpha
                    .then(|| context(w, h, depth, ChromaSampling::Cs400, None, q))
                    .transpose()
                    .map_err(encode_error)?,
            })
        } else {
            Frames::Eight(Planes {
                color: context(w, h, depth, color_sampling, Some(description), q)
                    .map_err(encode_error)?,
                alpha: alpha
                    .then(|| context(w, h, depth, ChromaSampling::Cs400, None, q))
                    .transpose()
                    .map_err(encode_error)?,
            })
        };
        Ok(Self {
            out: BufWriter::new(file),
            frames,
            size,
            channels: target.layout.channels() as usize,
            gray,
            sixteen,
            matrix,
            code: [primaries, transfer, matrix_code],
            next_row: 0,
            cancel,
        })
    }

    pub(super) fn write_rows(&mut self, first_row: u32, rows: &[u8]) -> Result<(), ExportError> {
        let bytes = if self.sixteen { 2 } else { 1 };
        let row_bytes = self.size.width as usize * self.channels * bytes;
        if first_row != self.next_row || !rows.len().is_multiple_of(row_bytes) {
            return Err(rows_out_of_order(first_row, self.next_row, rows.len()));
        }
        for row in rows.chunks_exact(row_bytes) {
            let y = self.next_row as usize;
            // Samples in [0, 1]: big-endian is never used for AVIF targets.
            let sample = |i: usize| -> f32 {
                if bytes == 1 {
                    f32::from(row[i]) / 255.0
                } else {
                    f32::from(u16::from_ne_bytes([row[2 * i], row[2 * i + 1]])) / 65535.0
                }
            };
            let (gray, channels, matrix) = (self.gray, self.channels, self.matrix);
            let pixel = |x: usize| -> ([f32; 3], Option<f32>) {
                let at = x * channels;
                let yuv = if gray {
                    [sample(at), 0.0, 0.0]
                } else {
                    to_yuv(matrix, [sample(at), sample(at + 1), sample(at + 2)])
                };
                let alpha = (channels == 2 || channels == 4).then(|| sample(at + channels - 1));
                (yuv, alpha)
            };
            match &mut self.frames {
                Frames::Eight(p) => fill_row(p, y, self.size.width as usize, gray, 255.0, pixel),
                Frames::Ten(p) => fill_row(p, y, self.size.width as usize, gray, 1023.0, pixel),
            }
            self.next_row += 1;
        }
        Ok(())
    }

    pub(super) fn finish(mut self) -> Result<Vec<ExportNotice>, ExportError> {
        if self.next_row != self.size.height {
            return Err(ExportError::Encode(format!(
                "{} rows written out of {}",
                self.next_row, self.size.height
            )));
        }
        if self.cancel.is_cancelled() {
            return Err(ExportError::Cancelled);
        }
        let (color, alpha) = match self.frames {
            Frames::Eight(p) => encode_both(p)?,
            Frames::Ten(p) => encode_both(p)?,
        };
        let [primaries, transfer, matrix] = self.code;
        let mut avif = avif_serialize::Aviffy::new();
        avif.set_color_primaries(color_primaries(primaries))
            .set_transfer_characteristics(transfer_characteristics(transfer))
            .set_matrix_coefficients(matrix_coefficients(if self.gray { 2 } else { matrix }))
            .set_full_color_range(true)
            .set_monochrome(self.gray);
        let depth = if self.sixteen { 10 } else { 8 };
        let file = avif.to_vec(
            &color,
            alpha.as_deref(),
            self.size.width,
            self.size.height,
            depth,
        );
        self.out.write_all(&file)?;
        self.out.flush()?;
        Ok(Vec::new())
    }
}

/// A full-range color in [0, 1] as (Y, U, V) in [0, 1] (chroma centered on ½), with the
/// matrix's `Kr` and `Kb`.
fn to_yuv((kr, kb): (f32, f32), [r, g, b]: [f32; 3]) -> [f32; 3] {
    let y = kr * r + (1.0 - kr - kb) * g + kb * b;
    [
        y,
        (b - y) / (2.0 * (1.0 - kb)) + 0.5,
        (r - y) / (2.0 * (1.0 - kr)) + 0.5,
    ]
}

/// Write row `y` of the frames from `pixel(x)`: (Y, U, V) and alpha in [0, 1].
fn fill_row<T: Pixel>(
    planes: &mut Planes<T>,
    y: usize,
    width: usize,
    gray: bool,
    max: f32,
    pixel: impl Fn(usize) -> ([f32; 3], Option<f32>),
) {
    let code = |v: f32| T::cast_from((v.clamp(0.0, 1.0) * max).round() as u16);
    // Row `y` of a plane: its first `width` samples (planes are padded around).
    fn row<T: Pixel>(plane: &mut rav1e::prelude::Plane<T>, y: usize, width: usize) -> &mut [T] {
        let stride = plane.cfg.stride;
        &mut plane.data_origin_mut()[y * stride..y * stride + width]
    }
    let [py, pu, pv] = &mut planes.color.1.planes;
    let row_y = row(py, y, width);
    let (mut row_u, mut row_v) = if gray {
        (None, None)
    } else {
        (Some(row(pu, y, width)), Some(row(pv, y, width)))
    };
    let mut row_a = planes
        .alpha
        .as_mut()
        .map(|(_, frame)| row(&mut frame.planes[0], y, width));
    for x in 0..width {
        let ([vy, vu, vv], alpha) = pixel(x);
        row_y[x] = code(vy);
        if let (Some(u), Some(v)) = (row_u.as_deref_mut(), row_v.as_deref_mut()) {
            u[x] = code(vu);
            v[x] = code(vv);
        }
        if let (Some(a), Some(value)) = (row_a.as_deref_mut(), alpha) {
            a[x] = code(value);
        }
    }
}

/// A rav1e context for one still frame, and its empty frame.
fn context<T: Pixel>(
    width: usize,
    height: usize,
    depth: usize,
    chroma_sampling: ChromaSampling,
    color_description: Option<ColorDescription>,
    quantizer: u8,
) -> Result<(Context<T>, Frame<T>), rav1e::InvalidConfig> {
    let mut speed_settings = SpeedSettings::from_preset(SPEED);
    // One frame: no references, nothing to look ahead to.
    speed_settings.multiref = false;
    speed_settings.rdo_lookahead_frames = 1;
    speed_settings.scene_detection_mode = SceneDetectionSpeed::None;

    // Tiles are encoded in parallel: one per core, none smaller than 128 × 128 (as ravif).
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let tiles = threads.min(width * height / (128 * 128)).max(1);
    let config = Config::new().with_encoder_config(EncoderConfig {
        width,
        height,
        bit_depth: depth,
        chroma_sampling,
        chroma_sample_position: ChromaSamplePosition::Unknown,
        pixel_range: PixelRange::Full,
        color_description,
        still_picture: true,
        quantizer: usize::from(quantizer),
        min_quantizer: quantizer,
        bitrate: 0,
        time_base: Rational::new(1, 1),
        tiles,
        speed_settings,
        ..EncoderConfig::default()
    });
    let context = config.new_context()?;
    let frame = context.new_frame();
    Ok((context, frame))
}

/// Encode the color frame and the alpha frame (in parallel): their AV1 data.
fn encode_both<T: Pixel>(planes: Planes<T>) -> Result<(Vec<u8>, Option<Vec<u8>>), ExportError> {
    let Planes { color, alpha } = planes;
    std::thread::scope(|scope| {
        let alpha = alpha.map(|alpha| scope.spawn(move || encode(alpha)));
        let color = encode(color)?;
        let alpha = match alpha {
            Some(handle) => Some(
                handle
                    .join()
                    .map_err(|_| ExportError::Encode("the AV1 encoder panicked".to_owned()))??,
            ),
            None => None,
        };
        Ok((color, alpha))
    })
}

fn encode<T: Pixel>((mut context, frame): (Context<T>, Frame<T>)) -> Result<Vec<u8>, ExportError> {
    let error = |e: EncoderStatus| ExportError::Encode(format!("AV1: {e}"));
    context.send_frame(frame).map_err(error)?;
    context.flush();
    let mut out = Vec::new();
    loop {
        match context.receive_packet() {
            Ok(mut packet) => {
                if packet.frame_type == FrameType::KEY {
                    out.append(&mut packet.data);
                }
            }
            Err(EncoderStatus::Encoded) => {}
            Err(EncoderStatus::LimitReached) => break,
            Err(e) => return Err(error(e)),
        }
    }
    Ok(out)
}

fn color_primaries(code: u8) -> avif_serialize::constants::ColorPrimaries {
    use avif_serialize::constants::ColorPrimaries as P;
    match code {
        1 => P::Bt709,
        9 => P::Bt2020,
        12 => P::DisplayP3,
        _ => P::Unspecified,
    }
}

fn transfer_characteristics(code: u8) -> avif_serialize::constants::TransferCharacteristics {
    use avif_serialize::constants::TransferCharacteristics as T;
    match code {
        1 => T::Bt709,
        8 => T::Linear,
        13 => T::Srgb,
        16 => T::Smpte2084,
        18 => T::Hlg,
        _ => T::Unspecified,
    }
}

fn matrix_coefficients(code: u8) -> avif_serialize::constants::MatrixCoefficients {
    use avif_serialize::constants::MatrixCoefficients as M;
    match code {
        0 => M::Rgb,
        1 => M::Bt709,
        9 => M::Bt2020Ncl,
        _ => M::Unspecified,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_maps_to_quantizers_like_ravif() {
        assert_eq!(quantizer(100), 0);
        assert!(quantizer(80) < quantizer(50));
        assert_eq!(quantizer(0), quantizer(1));
    }

    #[test]
    fn yuv_of_gray_has_neutral_chroma() {
        let [y, u, v] = to_yuv((0.2126, 0.0722), [0.4, 0.4, 0.4]);
        assert!((y - 0.4).abs() < 1e-6 && (u - 0.5).abs() < 1e-6 && (v - 0.5).abs() < 1e-6);
        // Red: Y is Kr, Cr at its top.
        let [y, _, v] = to_yuv((0.2126, 0.0722), [1.0, 0.0, 0.0]);
        assert!((y - 0.2126).abs() < 1e-6 && (v - 1.0).abs() < 1e-6);
    }
}
