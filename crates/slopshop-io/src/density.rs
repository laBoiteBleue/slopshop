//! The resolution image files declare, in pixels per inch (ADR 0028): JPEG's JFIF density or
//! EXIF resolution, PNG's `pHYs`, TIFF's `XResolution`, Photoshop's resolution resource. Read
//! from the header alone, apart from the pixels. Only the horizontal resolution is kept
//! (non-square pixels are rare; Photoshop keeps one resolution too).

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use slopshop_core::document::valid_resolution;

const CM_PER_INCH: f64 = 2.54;
/// Where a JPEG's APP0 and APP1 segments are: the file's first bytes.
const JPEG_HEAD: u64 = 64 * 1024;

/// The resolution `path` declares, if any and in range. `head`: the file's first bytes.
pub(crate) fn of_file(path: &Path, head: &[u8]) -> Option<f64> {
    // Camera RAW files are TIFF containers whose resolution means nothing.
    let ppi = if crate::raw::is_raw(path) {
        None
    } else if head.starts_with(&[0xFF, 0xD8]) {
        jpeg(path)
    } else if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        png(path)
    } else if head.starts_with(b"II*\0") || head.starts_with(b"MM\0*") {
        tiff(path)
    } else if crate::psd::is_psd(head) {
        crate::psd::resolution(path)
    } else {
        None
    };
    ppi.filter(|&ppi| valid_resolution(ppi))
}

fn jpeg(path: &Path) -> Option<f64> {
    let mut head = Vec::new();
    File::open(path)
        .ok()?
        .take(JPEG_HEAD)
        .read_to_end(&mut head)
        .ok()?;
    jpeg_segments(&head)
}

/// JFIF's density when it gives one, else EXIF's resolution, from a JPEG's first bytes.
pub(crate) fn jpeg_segments(jpeg: &[u8]) -> Option<f64> {
    let mut at = 2;
    let mut exif = None;
    // Markers until the image data: each segment is FF, its marker and a big-endian length.
    while at + 4 <= jpeg.len() && jpeg[at] == 0xFF {
        let marker = jpeg[at + 1];
        let len = usize::from(u16::from_be_bytes([jpeg[at + 2], jpeg[at + 3]]));
        let data = jpeg.get(at + 4..at + 2 + len)?;
        match marker {
            0xE0 if data.starts_with(b"JFIF\0") && data.len() >= 12 => {
                let x = f64::from(u16::from_be_bytes([data[8], data[9]]));
                match data[7] {
                    1 => return Some(x),
                    2 => return Some(x * CM_PER_INCH),
                    // 0: an aspect ratio only.
                    _ => {}
                }
            }
            0xE1 if data.starts_with(b"Exif\0\0") => exif = exif_resolution(&data[6..]),
            // Start of scan: no more metadata.
            0xDA => break,
            _ => {}
        }
        at += 2 + len;
    }
    exif
}

/// EXIF's `XResolution` and `ResolutionUnit` (IFD0 of the TIFF structure `tiff` holds).
pub(crate) fn exif_resolution(tiff: &[u8]) -> Option<f64> {
    let big = match tiff.get(..4)? {
        b"MM\0*" => true,
        b"II*\0" => false,
        _ => return None,
    };
    let u16_at = |at: usize| -> Option<u16> {
        let b = tiff.get(at..at + 2)?;
        Some(if big {
            u16::from_be_bytes([b[0], b[1]])
        } else {
            u16::from_le_bytes([b[0], b[1]])
        })
    };
    let u32_at = |at: usize| -> Option<u32> {
        let b = tiff.get(at..at + 4)?;
        let b = [b[0], b[1], b[2], b[3]];
        Some(if big {
            u32::from_be_bytes(b)
        } else {
            u32::from_le_bytes(b)
        })
    };
    let ifd = usize::try_from(u32_at(4)?).ok()?;
    let entries = usize::from(u16_at(ifd)?);
    let (mut resolution, mut unit) = (None, 2);
    for i in 0..entries {
        let entry = ifd + 2 + 12 * i;
        match u16_at(entry)? {
            // XResolution: a RATIONAL, stored at an offset.
            0x011A => {
                let at = usize::try_from(u32_at(entry + 8)?).ok()?;
                let (num, den) = (u32_at(at)?, u32_at(at + 4)?);
                if den != 0 {
                    resolution = Some(f64::from(num) / f64::from(den));
                }
            }
            // ResolutionUnit: a SHORT, in the entry (2 inches, 3 centimeters, 1 none).
            0x0128 => unit = u16_at(entry + 8)?,
            _ => {}
        }
    }
    match unit {
        2 => resolution,
        3 => resolution.map(|r| r * CM_PER_INCH),
        _ => None,
    }
}

/// `pHYs`: pixels per meter when its unit is the meter.
fn png(path: &Path) -> Option<f64> {
    let reader = png::Decoder::new(BufReader::new(File::open(path).ok()?))
        .read_info()
        .ok()?;
    let dims = reader.info().pixel_dims?;
    (dims.unit == png::Unit::Meter).then(|| f64::from(dims.xppu) * CM_PER_INCH / 100.0)
}

fn tiff(path: &Path) -> Option<f64> {
    use tiff::decoder::{Decoder, ifd::Value};
    use tiff::tags::Tag;
    let mut decoder = Decoder::new(BufReader::new(File::open(path).ok()?)).ok()?;
    let x = match decoder.find_tag(Tag::XResolution).ok()?? {
        Value::Rational(n, d) if d != 0 => f64::from(n) / f64::from(d),
        Value::Double(x) => x,
        _ => return None,
    };
    let unit = decoder
        .find_tag_unsigned::<u16>(Tag::ResolutionUnit)
        .ok()
        .flatten()
        .unwrap_or(2);
    match unit {
        2 => Some(x),
        3 => Some(x * CM_PER_INCH),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A JPEG's first segments: SOI, then `segments` (marker, payload), then SOS.
    fn jpeg(segments: &[(u8, Vec<u8>)]) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xD8];
        for (marker, payload) in segments {
            bytes.extend([0xFF, *marker]);
            bytes.extend(((payload.len() + 2) as u16).to_be_bytes());
            bytes.extend(payload);
        }
        bytes.extend([0xFF, 0xDA, 0, 2]);
        bytes
    }

    fn jfif(unit: u8, x: u16) -> Vec<u8> {
        let mut data = b"JFIF\0\x01\x02".to_vec();
        data.push(unit);
        data.extend(x.to_be_bytes());
        data.extend(x.to_be_bytes());
        data.extend([0, 0]);
        data
    }

    /// An EXIF payload (little-endian TIFF) with `XResolution` = `num`/`den` and a unit.
    fn exif(num: u32, den: u32, unit: u16) -> Vec<u8> {
        let mut t = b"Exif\0\0II*\0".to_vec();
        t.extend(8u32.to_le_bytes());
        t.extend(2u16.to_le_bytes());
        // XResolution, RATIONAL, 1 value, at offset 8 + 2 + 24 + 4 = 38.
        t.extend(0x011Au16.to_le_bytes());
        t.extend(5u16.to_le_bytes());
        t.extend(1u32.to_le_bytes());
        t.extend(38u32.to_le_bytes());
        // ResolutionUnit, SHORT, 1 value, in the entry.
        t.extend(0x0128u16.to_le_bytes());
        t.extend(3u16.to_le_bytes());
        t.extend(1u32.to_le_bytes());
        t.extend(u32::from(unit).to_le_bytes());
        t.extend(0u32.to_le_bytes());
        t.extend(num.to_le_bytes());
        t.extend(den.to_le_bytes());
        t
    }

    #[test]
    fn jpeg_resolution_from_jfif_then_exif() {
        assert_eq!(jpeg_segments(&jpeg(&[(0xE0, jfif(1, 300))])), Some(300.0));
        let per_cm = jpeg_segments(&jpeg(&[(0xE0, jfif(2, 100))])).unwrap();
        assert!((per_cm - 254.0).abs() < 1e-9);
        // JFIF with an aspect ratio only: EXIF says.
        assert_eq!(
            jpeg_segments(&jpeg(&[(0xE0, jfif(0, 1)), (0xE1, exif(240, 1, 2))])),
            Some(240.0)
        );
        assert_eq!(
            jpeg_segments(&jpeg(&[(0xE1, exif(7200, 100, 2))])),
            Some(72.0)
        );
        assert_eq!(jpeg_segments(&jpeg(&[(0xE1, exif(300, 1, 1))])), None);
        assert_eq!(jpeg_segments(&jpeg(&[])), None);
        // Truncated: nothing, no panic.
        assert_eq!(jpeg_segments(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00]), None);
    }
}
