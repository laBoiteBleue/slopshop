//! Camera RAW import (ADR 0023): the `slopshop-raw` helper, a separate executable, decodes the
//! file with rawler (LGPL-2.1, kept out of this crate, ADR 0006) and develops it as shot to
//! linear Rec.2020 floats, which this module reads from its output. Recognized by extension:
//! many RAW files are TIFF containers.
//!
//! The helper is found through `SLOPSHOP_RAW_WORKER`, else next to the running executable (or
//! one folder up, for test binaries in `target/*/deps`).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use slopshop_core::Size;
use slopshop_core::color::{AlphaMode, ChannelLayout, ColorSpace, SampleType};

use crate::orient::Orientation;
use crate::{Decoded, ImportError, ImportWarning, check_budget};

/// The extensions of the camera RAW formats rawler reads (DNG included).
pub(crate) const EXTENSIONS: [&str; 24] = [
    "cr2", "cr3", "crw", "nef", "nrw", "arw", "srf", "sr2", "raf", "orf", "rw2", "rwl", "pef",
    "srw", "dng", "3fr", "fff", "iiq", "x3f", "erf", "kdc", "dcr", "mrw", "mos",
];

/// The helper's flag: the file had no color matrix.
const NO_COLOR_MATRIX: u32 = 1;

/// Whether `path` has a camera RAW extension.
pub(crate) fn is_raw(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// The helper's path, if it can be found.
fn helper() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("SLOPSHOP_RAW_WORKER") {
        return Some(PathBuf::from(path));
    }
    let name = format!("slopshop-raw{}", std::env::consts::EXE_SUFFIX);
    let exe = std::env::current_exe().ok()?;
    let folder = exe.parent()?;
    [Some(folder), folder.parent()]
        .into_iter()
        .flatten()
        .map(|dir| dir.join(&name))
        .find(|path| path.is_file())
}

pub(crate) fn decode(path: &Path) -> Result<Decoded, ImportError> {
    let helper = helper().ok_or_else(|| {
        ImportError::Decode("camera RAW: the slopshop-raw helper was not found".into())
    })?;
    let mut child = Command::new(helper)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let decoded = child
        .stdout
        .take()
        .ok_or_else(|| ImportError::Decode("camera RAW: no output".into()))
        .and_then(read);
    // The helper's message explains a failure better than a short read.
    let output = child.wait_with_output()?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(ImportError::Decode(format!("camera RAW: {message}")));
    }
    decoded
}

/// The helper's output (see `slopshop-raw`): its header, then the samples.
fn read(mut input: impl Read) -> Result<Decoded, ImportError> {
    let corrupt = |what: &str| ImportError::Decode(format!("camera RAW: {what}"));
    let mut header = [0u8; 24];
    input
        .read_exact(&mut header)
        .map_err(|_| corrupt("no image"))?;
    if &header[..8] != b"SLOPRAW1" {
        return Err(corrupt("unexpected output"));
    }
    let u32_at =
        |i: usize| u32::from_le_bytes([header[i], header[i + 1], header[i + 2], header[i + 3]]);
    let u16_at = |i: usize| u16::from_le_bytes([header[i], header[i + 1]]);
    let (width, height) = (u32_at(8), u32_at(12));
    let orientation = Orientation::from_exif(u16_at(16));
    let (layout, channels) = match u16_at(18) {
        1 => (ChannelLayout::Gray, 1u64),
        3 => (ChannelLayout::Rgb, 3),
        other => return Err(corrupt(&format!("{other} channels"))),
    };
    let flags = u32_at(20);
    check_budget(
        width,
        height,
        layout,
        SampleType::F32,
        (channels * 4) as u32,
    )?;
    let size = Size::new(width, height);
    let length = size.pixel_count() * channels * 4;
    let mut pixels = Vec::new();
    input.take(length).read_to_end(&mut pixels)?;
    if (pixels.len() as u64) < length {
        return Err(corrupt("truncated output"));
    }
    // The helper writes little-endian; the engine reads native order.
    if cfg!(target_endian = "big") {
        for sample in pixels.as_chunks_mut::<4>().0 {
            sample.reverse();
        }
    }
    let mut warnings = Vec::new();
    if flags & NO_COLOR_MATRIX != 0 {
        warnings.push(ImportWarning::ColorInfoUnsupported);
    }
    Ok(Decoded {
        size,
        layout,
        sample: SampleType::F32,
        alpha: AlphaMode::Straight,
        icc: None,
        space: Some(ColorSpace::LINEAR_REC2020),
        orientation,
        pixels,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(channels: u16, flags: u32, samples: &[f32]) -> Vec<u8> {
        let mut out = b"SLOPRAW1".to_vec();
        out.extend(2u32.to_le_bytes());
        out.extend(1u32.to_le_bytes());
        out.extend(6u16.to_le_bytes());
        out.extend(channels.to_le_bytes());
        out.extend(flags.to_le_bytes());
        for v in samples {
            out.extend(v.to_le_bytes());
        }
        out
    }

    #[test]
    fn the_helpers_output_becomes_linear_rec2020_floats() {
        let decoded = read(&output(3, 0, &[0.5, 0.25, 0.125, 1.5, -0.1, 0.0])[..]).unwrap();
        assert_eq!(
            (decoded.size, decoded.layout, decoded.sample),
            (Size::new(2, 1), ChannelLayout::Rgb, SampleType::F32)
        );
        assert_eq!(decoded.space, Some(ColorSpace::LINEAR_REC2020));
        assert_eq!(decoded.orientation, Orientation::Rotate90);
        assert_eq!(&decoded.pixels[..4], &0.5f32.to_ne_bytes());
        assert!(decoded.warnings.is_empty());
        let gray = read(&output(1, NO_COLOR_MATRIX, &[0.5, 0.5])[..]).unwrap();
        assert_eq!(gray.layout, ChannelLayout::Gray);
        assert_eq!(gray.warnings, [ImportWarning::ColorInfoUnsupported]);
    }

    #[test]
    fn short_or_foreign_output_is_an_error() {
        let full = output(3, 0, &[0.5; 6]);
        assert!(read(&full[..full.len() - 4]).is_err());
        assert!(read(&b"something else entirely"[..]).is_err());
        assert!(read(&output(2, 0, &[0.5; 4])[..]).is_err());
    }

    #[test]
    fn raw_files_are_known_by_extension() {
        assert!(is_raw(Path::new("IMG_0001.CR2")));
        assert!(is_raw(Path::new("photo.dng")));
        assert!(!is_raw(Path::new("photo.tif")));
    }

    /// With the helper built (`cargo build -p slopshop-raw`), a DNG opens through it.
    #[test]
    fn a_dng_opens_through_the_helper_when_it_is_built() {
        if helper().is_none() {
            eprintln!("slopshop-raw is not built: skipped");
            return;
        }
        let dng = Path::new(env!("CARGO_MANIFEST_DIR")).join("../slopshop-raw/fixtures/tiny.dng");
        let imported = crate::open_image(&dng).unwrap();
        assert_eq!(imported.image.size(), Size::new(32, 24));
        assert_eq!(
            imported.image.format().color_space,
            ColorSpace::LINEAR_REC2020
        );
    }
}
