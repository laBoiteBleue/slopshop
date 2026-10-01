//! `slopshop-raw <FILE>`: decodes a camera RAW file with rawler and writes it developed "as
//! shot" to standard output, for SlopShop's importer (ADR 0023). A separate process, so that
//! rawler (LGPL-2.1) stays a replaceable component outside the editor's binaries (ADR 0006), and
//! a crash in a decoder fails one import, not the editor.
//!
//! Development: rawler scales the sensor values (black and white levels) and demosaics them
//! (PPG for Bayer sensors, bilinear for X-Trans); then the camera's white balance as shot,
//! highlights clipped where a channel saturates (as dcraw does: saturated areas turn white
//! rather than magenta), and the camera's color matrix to linear Rec.2020, the editor's working
//! space, without clipping the gamut. No tone curve: the image is scene-linear, 1.0 being the
//! sensor's white.
//!
//! Output: `SLOPRAW1`, then width and height (u32), the EXIF orientation (u16), the channel
//! count (u16: 3, or 1 for a monochrome sensor) and flags (u32: bit 0, no color matrix: the
//! camera's colors were taken as Rec.2020), all little-endian, then width × height × channels
//! `f32` little-endian, rows top to bottom. On failure: a message on standard error, exit 1.

use std::io::{BufWriter, Write};
use std::path::Path;
use std::process::ExitCode;

use rawler::imgop::develop::{Intermediate, ProcessingStep, RawDevelop};
use rawler::imgop::xyz::Illuminant;
use slopshop_core::color::{Mat3, RgbPrimaries, mat_inverse, mat_mul};

/// Bit 0 of the flags: the file has no color matrix.
const NO_COLOR_MATRIX: u32 = 1;

/// A developed image.
struct Developed {
    width: u32,
    height: u32,
    orientation: u16,
    channels: u16,
    flags: u32,
    samples: Vec<f32>,
}

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let [input] = args.as_slice() else {
        eprintln!("usage: slopshop-raw <FILE>");
        return ExitCode::from(2);
    };
    let developed = match develop(Path::new(input)) {
        Ok(developed) => developed,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };
    match write(&developed, &mut BufWriter::new(std::io::stdout().lock())) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("cannot write the image: {e}");
            ExitCode::FAILURE
        }
    }
}

fn write(developed: &Developed, out: &mut impl Write) -> std::io::Result<()> {
    out.write_all(b"SLOPRAW1")?;
    out.write_all(&developed.width.to_le_bytes())?;
    out.write_all(&developed.height.to_le_bytes())?;
    out.write_all(&developed.orientation.to_le_bytes())?;
    out.write_all(&developed.channels.to_le_bytes())?;
    out.write_all(&developed.flags.to_le_bytes())?;
    // In bounded chunks: the image can be hundreds of megabytes.
    let mut bytes = Vec::with_capacity(1 << 16);
    for chunk in developed.samples.chunks(1 << 14) {
        bytes.clear();
        for v in chunk {
            bytes.extend(v.to_le_bytes());
        }
        out.write_all(&bytes)?;
    }
    out.flush()
}

fn develop(path: &Path) -> Result<Developed, String> {
    let raw = rawler::decode_file(path).map_err(|e| format!("cannot decode: {e}"))?;
    // rawler's own white balance, calibration and sRGB steps are left out: ours keep the gamut.
    let steps = [
        ProcessingStep::Rescale,
        ProcessingStep::Demosaic,
        ProcessingStep::FujiRotate,
        ProcessingStep::CropActiveArea,
        ProcessingStep::CropDefault,
    ];
    let intermediate = RawDevelop::new_with(&steps)
        .develop_intermediate(&raw)
        .map_err(|e| format!("cannot develop: {e}"))?;
    let orientation = raw.orientation.to_u16();
    let size = |w: usize, h: usize| -> Result<(u32, u32), String> {
        Ok((
            u32::try_from(w).map_err(|_| "image too large")?,
            u32::try_from(h).map_err(|_| "image too large")?,
        ))
    };
    match intermediate {
        Intermediate::Monochrome(pixels) => {
            let (width, height) = size(pixels.width, pixels.height)?;
            Ok(Developed {
                width,
                height,
                orientation,
                channels: 1,
                flags: 0,
                samples: pixels.data,
            })
        }
        Intermediate::ThreeColor(pixels) => {
            let (width, height) = size(pixels.width, pixels.height)?;
            let (cam_to_rgb, flags) = camera_to_rec2020(&raw);
            let wb = white_balance(raw.wb_coeffs);
            let samples = pixels
                .data
                .iter()
                .flat_map(|&pixel| calibrate(pixel, wb, &cam_to_rgb))
                .collect();
            Ok(Developed {
                width,
                height,
                orientation,
                channels: 3,
                flags,
                samples,
            })
        }
        Intermediate::FourColor(_) => {
            Err("four-color sensors (CYGM, RGBE) are not supported yet".to_owned())
        }
    }
}

/// The as-shot white balance, normalized so that the smallest multiplier is 1 (as dcraw does):
/// a channel reaching 1.0 after it is saturated. Files without one keep the camera's colors.
fn white_balance(coefficients: [f32; 4]) -> [f64; 3] {
    let wb = [0, 1, 2].map(|i| f64::from(coefficients[i]));
    if wb.iter().any(|v| !v.is_finite() || *v <= 0.0) {
        return [1.0; 3];
    }
    let smallest = wb.iter().copied().fold(f64::MAX, f64::min);
    wb.map(|v| v / smallest)
}

/// Camera RGB to linear Rec.2020 (D65): the inverse of the camera's XYZ → camera matrix applied
/// to Rec.2020, its rows normalized so that the camera's white stays white. Matrices measured
/// under another illuminant are used as they are (an approximation); without any, the camera's
/// colors are taken as Rec.2020 (flagged).
fn camera_to_rec2020(raw: &rawler::RawImage) -> (Mat3, u32) {
    let found = raw.color_matrix_find_first([
        Illuminant::D65,
        Illuminant::D55,
        Illuminant::D50,
        Illuminant::D75,
        Illuminant::Daylight,
        Illuminant::Flash,
        Illuminant::A,
    ]);
    let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let Some((_, matrix)) = found.filter(|(_, m)| m.len() == 9) else {
        return (identity, NO_COLOR_MATRIX);
    };
    let xyz_to_camera: Mat3 =
        [0, 1, 2].map(|row| [0, 1, 2].map(|col| f64::from(matrix[row * 3 + col])));
    let mut rgb_to_camera = mat_mul(&xyz_to_camera, &RgbPrimaries::REC2020.to_xyz());
    for row in &mut rgb_to_camera {
        let sum: f64 = row.iter().sum();
        if sum.abs() > f64::EPSILON {
            for v in row.iter_mut() {
                *v /= sum;
            }
        }
    }
    match mat_inverse(&rgb_to_camera) {
        Some(camera_to_rgb) => (camera_to_rgb, 0),
        None => (identity, NO_COLOR_MATRIX),
    }
}

/// One pixel: white-balanced, clipped at the sensor's white, to Rec.2020.
fn calibrate(pixel: [f32; 3], wb: [f64; 3], cam_to_rgb: &Mat3) -> [f32; 3] {
    let camera = [0, 1, 2].map(|i| (f64::from(pixel[i]) * wb[i]).min(1.0));
    cam_to_rgb.map(|row| (row[0] * camera[0] + row[1] * camera[1] + row[2] * camera[2]) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_balance_is_normalized_to_its_smallest_multiplier() {
        assert_eq!(white_balance([2.0, 1.0, 1.5, 1.0]), [2.0, 1.0, 1.5]);
        assert_eq!(white_balance([4.0, 2.0, 3.0, 2.0]), [2.0, 1.0, 1.5]);
        assert_eq!(white_balance([f32::NAN, 1.0, 1.0, 1.0]), [1.0; 3]);
    }

    #[test]
    fn saturated_channels_clip_to_white() {
        let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        // A saturated pixel stays white whatever the balance.
        assert_eq!(
            calibrate([1.0, 1.0, 1.0], [2.0, 1.0, 1.5], &identity),
            [1.0, 1.0, 1.0]
        );
        assert_eq!(
            calibrate([0.25, 0.5, 0.2], [2.0, 1.0, 1.5], &identity),
            [0.5, 0.5, 0.3]
        );
    }

    #[test]
    fn the_output_header_and_samples() {
        let developed = Developed {
            width: 2,
            height: 1,
            orientation: 6,
            channels: 1,
            flags: NO_COLOR_MATRIX,
            samples: vec![0.5, 1.5],
        };
        let mut out = Vec::new();
        write(&developed, &mut out).unwrap();
        assert_eq!(&out[..8], b"SLOPRAW1");
        assert_eq!(out.len(), 8 + 4 + 4 + 2 + 2 + 4 + 8);
        assert_eq!(&out[16..18], &6u16.to_le_bytes());
        assert_eq!(&out[24..28], &0.5f32.to_le_bytes());
    }
}
