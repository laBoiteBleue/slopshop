//! Portable Float Map import: `PF` (RGB) or `Pf` (gray), then the width, the height and a
//! scale whose sign gives the byte order (negative: little-endian), separated by whitespace,
//! one whitespace character, then 32-bit float rows from the bottom up. Linear, by convention
//! (resolved like the other float formats).

use std::io::{BufReader, Read};
use std::path::Path;

use slopshop_core::Size;
use slopshop_core::color::{AlphaMode, ChannelLayout, SampleType};

use crate::orient::Orientation;
use crate::{Decoded, ImportError, check_budget};

/// Whether `head` starts like a PFM file.
pub(crate) fn is_pfm(head: &[u8]) -> bool {
    matches!(head, [b'P', b'F' | b'f', c, ..] if c.is_ascii_whitespace())
}

pub(crate) fn decode(path: &Path) -> Result<Decoded, ImportError> {
    let mut input = BufReader::new(std::fs::File::open(path)?);
    let corrupt = |what: &str| ImportError::Decode(format!("PFM: {what}"));
    // The header: four tokens, the last followed by exactly one whitespace byte.
    let mut tokens = Vec::new();
    let mut token = Vec::new();
    while tokens.len() < 4 {
        let mut byte = [0u8];
        if input.read(&mut byte)? == 0 {
            return Err(corrupt("truncated header"));
        }
        if byte[0].is_ascii_whitespace() {
            if !token.is_empty() {
                tokens.push(String::from_utf8_lossy(&token).into_owned());
                token.clear();
            }
        } else if token.len() < 32 {
            token.push(byte[0]);
        } else {
            return Err(corrupt("header token too long"));
        }
    }
    let layout = match tokens[0].as_str() {
        "PF" => ChannelLayout::Rgb,
        "Pf" => ChannelLayout::Gray,
        _ => return Err(ImportError::Unrecognized),
    };
    let number = |t: &str| t.parse::<u32>().map_err(|_| corrupt("bad size"));
    let (width, height) = (number(&tokens[1])?, number(&tokens[2])?);
    let scale: f32 = tokens[3].parse().map_err(|_| corrupt("bad scale"))?;
    if !scale.is_finite() || scale == 0.0 {
        return Err(corrupt("bad scale"));
    }
    let little = scale < 0.0;
    let channels = if layout == ChannelLayout::Rgb { 3 } else { 1 };
    check_budget(width, height, layout, SampleType::F32, channels * 4)?;
    let row_bytes = width as usize * channels as usize * 4;
    let mut pixels = vec![0u8; row_bytes * height as usize];
    // Bottom row first: each stored row goes to its place from the top.
    for stored in 0..height as usize {
        let at = (height as usize - 1 - stored) * row_bytes;
        let row = &mut pixels[at..at + row_bytes];
        input
            .read_exact(row)
            .map_err(|_| corrupt("truncated pixel data"))?;
        for sample in row.as_chunks_mut::<4>().0 {
            let v = if little {
                f32::from_le_bytes(*sample)
            } else {
                f32::from_be_bytes(*sample)
            };
            *sample = v.to_ne_bytes();
        }
    }
    Ok(Decoded {
        size: Size::new(width, height),
        layout,
        sample: SampleType::F32,
        alpha: AlphaMode::Straight,
        icc: None,
        space: None,
        orientation: Orientation::Normal,
        pixels,
        warnings: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("slopshop-pfm-{}-{name}", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn gray_big_endian_rows_come_back_top_down() {
        // 2 × 2 gray, big-endian (positive scale), bottom row first.
        let mut bytes = b"Pf\n2 2\n1.0\n".to_vec();
        for v in [3.0f32, 4.0, 1.0, 2.0] {
            bytes.extend(v.to_be_bytes());
        }
        let path = write("gray.pfm", &bytes);
        assert!(is_pfm(&bytes));
        let decoded = decode(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(decoded.layout, ChannelLayout::Gray);
        let values: Vec<f32> = decoded
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_ne_bytes(*b))
            .collect();
        assert_eq!(values, [1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn damaged_files_are_errors() {
        for (name, bytes) in [
            ("short.pfm", b"PF\n4 4\n-1.0\n\0\0\0\0".to_vec()),
            (
                "scale.pfm",
                b"PF\n1 1\n0\n\0\0\0\0\0\0\0\0\0\0\0\0".to_vec(),
            ),
            ("size.pfm", b"PF\nx 1\n-1\n".to_vec()),
            ("empty.pfm", b"PF\n0 1\n-1\n".to_vec()),
        ] {
            let path = write(name, &bytes);
            assert!(decode(&path).is_err(), "{name}");
            std::fs::remove_file(&path).ok();
        }
        assert!(!is_pfm(b"P6\n"));
    }
}
