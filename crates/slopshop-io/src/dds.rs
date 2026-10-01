//! Uncompressed DDS import: the legacy header's RGB pixel formats with 8-bit channels (24-bit
//! BGR, 32-bit BGRA, RGBA, BGRX…), which the `image` crate does not read (it decodes the DXT1,
//! DXT3 and DXT5 block formats only, and keeps doing so). The top-level surface is read: mip
//! levels after it are ignored; the other faces of a cube map, or slices of a volume texture,
//! are reported ([`ImportWarning::FirstPageOnly`]). sRGB by convention, straight alpha.

use std::io::{BufReader, Read};
use std::path::Path;

use slopshop_core::Size;
use slopshop_core::color::{AlphaMode, ChannelLayout, SampleType};

use crate::orient::Orientation;
use crate::{Decoded, ImportError, ImportWarning, check_budget};

/// "DDS ", then the 124-byte DDS_HEADER.
const HEADER_BYTES: usize = 128;
/// DDS_PIXELFORMAT flags.
const DDPF_ALPHAPIXELS: u32 = 0x1;
const DDPF_FOURCC: u32 = 0x4;
const DDPF_RGB: u32 = 0x40;
/// dwCaps2 flags.
const DDSCAPS2_CUBEMAP: u32 = 0x200;
const DDSCAPS2_VOLUME: u32 = 0x20_0000;

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at + 4)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Whether `head` starts like an uncompressed RGB DDS file (no FourCC).
pub(crate) fn is_uncompressed_dds(head: &[u8]) -> bool {
    head.starts_with(b"DDS ")
        && u32_at(head, 4) == Some(124)
        && u32_at(head, 76) == Some(32)
        && u32_at(head, 80).is_some_and(|flags| flags & DDPF_FOURCC == 0 && flags & DDPF_RGB != 0)
}

pub(crate) fn decode(path: &Path) -> Result<Decoded, ImportError> {
    let mut input = BufReader::new(std::fs::File::open(path)?);
    let mut header = [0u8; HEADER_BYTES];
    input
        .read_exact(&mut header)
        .map_err(|_| corrupt("truncated header"))?;
    if !is_uncompressed_dds(&header) {
        return Err(ImportError::Unrecognized);
    }
    let field = |at: usize| u32_at(&header, at).unwrap_or(0);
    let (height, width) = (field(12), field(16));
    let flags = field(80);
    let bits = field(88);
    let masks = [field(92), field(96), field(100)];
    let alpha_mask = if flags & DDPF_ALPHAPIXELS != 0 {
        field(104)
    } else {
        0
    };
    let pixel_bytes = match bits {
        24 => 3,
        32 => 4,
        _ => return Err(unsupported(bits, masks, alpha_mask)),
    };
    // Each channel a whole byte of the pixel (little-endian), all different.
    let byte_of = |mask: u32| (0..pixel_bytes).find(|&i| mask == 0xff << (8 * i));
    let (Some(r), Some(g), Some(b)) = (byte_of(masks[0]), byte_of(masks[1]), byte_of(masks[2]))
    else {
        return Err(unsupported(bits, masks, alpha_mask));
    };
    let a = match alpha_mask {
        0 => None,
        mask => Some(byte_of(mask).ok_or_else(|| unsupported(bits, masks, alpha_mask))?),
    };
    let mut used = vec![r, g, b];
    used.extend(a);
    used.sort_unstable();
    used.dedup();
    if used.len() != 3 + usize::from(a.is_some()) {
        return Err(unsupported(bits, masks, alpha_mask));
    }

    let (layout, channels) = match a {
        Some(_) => (ChannelLayout::Rgba, 4),
        None => (ChannelLayout::Rgb, 3),
    };
    check_budget(width, height, layout, SampleType::U8, channels)?;
    let (w, h) = (width as usize, height as usize);
    // Rows are tightly packed (the pitch field is unreliable in older files).
    let mut row = vec![0u8; w * pixel_bytes];
    let mut pixels = Vec::with_capacity(w * h * channels as usize);
    for _ in 0..h {
        input
            .read_exact(&mut row)
            .map_err(|_| corrupt("truncated pixel data"))?;
        for px in row.chunks_exact(pixel_bytes) {
            pixels.extend([px[r], px[g], px[b]]);
            if let Some(a) = a {
                pixels.push(px[a]);
            }
        }
    }
    let mut warnings = Vec::new();
    if field(112) & (DDSCAPS2_CUBEMAP | DDSCAPS2_VOLUME) != 0 {
        warnings.push(ImportWarning::FirstPageOnly);
    }
    Ok(Decoded {
        size: Size::new(width, height),
        layout,
        sample: SampleType::U8,
        alpha: AlphaMode::Straight,
        icc: None,
        space: None,
        orientation: Orientation::Normal,
        pixels,
        warnings,
    })
}

fn corrupt(what: &str) -> ImportError {
    ImportError::Decode(format!("DDS: {what}"))
}

fn unsupported(bits: u32, masks: [u32; 3], alpha: u32) -> ImportError {
    ImportError::UnsupportedPixels(format!(
        "DDS {bits}-bit RGB, masks {:#x} {:#x} {:#x} {alpha:#x}",
        masks[0], masks[1], masks[2]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A legacy header for an uncompressed `width` × `height` surface.
    fn header(width: u32, height: u32, bits: u32, masks: [u32; 4], caps2: u32) -> Vec<u8> {
        let mut h = b"DDS ".to_vec();
        for v in [124, 0x100f, height, width, width * bits / 8, 0, 1] {
            h.extend(u32::to_le_bytes(v));
        }
        h.extend([0; 44]);
        let flags = DDPF_RGB | if masks[3] != 0 { DDPF_ALPHAPIXELS } else { 0 };
        for v in [32, flags, 0, bits] {
            h.extend(u32::to_le_bytes(v));
        }
        for m in masks {
            h.extend(m.to_le_bytes());
        }
        for v in [0x1000, caps2, 0, 0, 0] {
            h.extend(u32::to_le_bytes(v));
        }
        assert_eq!(h.len(), HEADER_BYTES);
        h
    }

    fn decode_bytes(name: &str, bytes: &[u8]) -> Result<Decoded, ImportError> {
        let path = std::env::temp_dir().join(format!("slopshop-dds-{}-{name}", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        let result = decode(&path);
        std::fs::remove_file(&path).ok();
        result
    }

    #[test]
    fn rgba_and_bgr_byte_orders_are_read() {
        // A8B8G8R8 (RGBA in memory), 2 × 1.
        let mut file = header(2, 1, 32, [0xff, 0xff00, 0xff_0000, 0xff00_0000], 0);
        file.extend([1, 2, 3, 4, 5, 6, 7, 8]);
        assert!(is_uncompressed_dds(&file));
        let decoded = decode_bytes("rgba.dds", &file).unwrap();
        assert_eq!(decoded.layout, ChannelLayout::Rgba);
        assert_eq!(decoded.pixels, [1, 2, 3, 4, 5, 6, 7, 8]);
        assert!(decoded.warnings.is_empty());

        // R8G8B8 (BGR in memory), a cube map: its first face, reported.
        let mut file = header(1, 2, 24, [0xff_0000, 0xff00, 0xff, 0], DDSCAPS2_CUBEMAP);
        file.extend([1, 2, 3, 4, 5, 6]);
        let decoded = decode_bytes("bgr.dds", &file).unwrap();
        assert_eq!(decoded.layout, ChannelLayout::Rgb);
        assert_eq!(decoded.pixels, [3, 2, 1, 6, 5, 4]);
        assert_eq!(decoded.warnings, [ImportWarning::FirstPageOnly]);
    }

    #[test]
    fn other_layouts_and_damaged_files_are_errors() {
        // 16-bit 565, masks sharing a byte, a truncated surface.
        let r5g6b5 = header(1, 1, 16, [0xf800, 0x7e0, 0x1f, 0], 0);
        let shared = header(1, 1, 32, [0xff, 0xff, 0xff00, 0], 0);
        let mut short = header(4, 4, 32, [0xff_0000, 0xff00, 0xff, 0xff00_0000], 0);
        short.extend([0; 10]);
        for (name, bytes) in [
            ("565.dds", r5g6b5),
            ("shared.dds", shared),
            ("short.dds", short),
        ] {
            assert!(decode_bytes(name, &bytes).is_err(), "{name}");
        }
        // Block-compressed files are left to the image crate.
        let mut dxt1 = header(4, 4, 0, [0; 4], 0);
        dxt1[80..84].copy_from_slice(&DDPF_FOURCC.to_le_bytes());
        assert!(!is_uncompressed_dds(&dxt1));
    }
}
