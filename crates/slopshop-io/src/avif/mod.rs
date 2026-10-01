//! AVIF import: the container read by our own code ([`container`]), each AV1 image decoded by
//! rav1d ([`ffi`], the only `unsafe` part), YUV converted to RGB ([`yuv`]).
//!
//! The primary image (a single AV1 image or a grid of tiles), with its alpha plane when it has
//! one (straight, or premultiplied with a `prem` reference), at its precision (8-bit, or
//! 10/12-bit as 16-bit), gray when monochrome. Color comes from the `colr` property (H.273 code
//! points or an ICC profile), else the AV1 sequence header; unspecified code points mean sRGB.
//! The clean aperture (`clap`) crops, rotations and mirrors (`irot`, `imir`) orient. An
//! animated file gives its still image, reported.

mod container;
mod ffi;
mod yuv;

use std::path::Path;

use slopshop_core::Size;
use slopshop_core::color::{AlphaMode, ChannelLayout, SampleType};

pub(crate) use self::container::is_avif;
use self::container::{Item, Properties, Transform};
use self::ffi::{Chroma, Picture};
use self::yuv::Target;
use crate::orient::Orientation;
use crate::{Decoded, ImportError, ImportWarning, check_budget, cicp_code_space};

pub(crate) fn decode(path: &Path) -> Result<Decoded, ImportError> {
    let decode_error = |e: String| ImportError::Decode(format!("AVIF: {e}"));
    let file = std::fs::read(path)?;
    let avif = container::parse(&file).map_err(decode_error)?;
    let (width, height) = image_size(&avif.primary).map_err(decode_error)?;
    // Before decoding: the largest import of this size (16-bit, with alpha).
    check_budget(width, height, ChannelLayout::Rgba, SampleType::U16, 16)?;
    let max_pixels = width.saturating_mul(height);

    // The first picture says the depth and whether the image is gray.
    let first = match avif.tiles.first() {
        Some(tile) => &tile.data,
        None => &avif.primary.data,
    };
    let first = ffi::decode(first, max_pixels).map_err(decode_error)?;
    let gray = first.chroma == Chroma::Monochrome;
    let sample = if first.depth == 8 {
        SampleType::U8
    } else {
        SampleType::U16
    };
    let has_alpha = avif.alpha.is_some();
    let layout = match (gray, has_alpha) {
        (false, false) => ChannelLayout::Rgb,
        (false, true) => ChannelLayout::Rgba,
        (true, false) => ChannelLayout::Gray,
        (true, true) => ChannelLayout::GrayAlpha,
    };
    let channels = layout.channels() as usize;
    let max = if sample == SampleType::U8 {
        255.0
    } else {
        65535.0
    };
    let mut samples = vec![0u16; width as usize * height as usize * channels];
    let properties = &avif.primary.properties;
    // The container's color description wins over the bitstream's.
    let (primaries, transfer, matrix, _) = properties.nclx.unwrap_or((
        first.primaries,
        first.transfer,
        first.matrix,
        first.full_range,
    ));
    let mut target = Target {
        out: &mut samples,
        width: width as usize,
        height: height as usize,
        channels,
        max,
        x0: 0,
        y0: 0,
    };
    let place_color = |picture: &Picture, target: &mut Target<'_>| {
        // Each picture's own range when the container does not say.
        let full = properties.nclx.map_or(picture.full_range, |c| c.3);
        yuv::color(picture, matrix, full, target)
    };
    for_each_picture(
        &avif.primary,
        &avif.tiles,
        Some(first),
        max_pixels,
        &mut target,
        &mut |picture, target| place_color(picture, target),
    )
    .map_err(decode_error)?;
    if let Some((alpha, tiles)) = &avif.alpha {
        for_each_picture(
            alpha,
            tiles,
            None,
            max_pixels,
            &mut target,
            &mut |p, target| {
                yuv::alpha(p, p.full_range, target);
                Ok(())
            },
        )
        .map_err(decode_error)?;
    }

    let (mut size, mut samples) = (Size::new(width, height), samples);
    if let Some(crop) = clean_aperture(properties, size) {
        samples = cropped(&samples, size, channels, crop);
        size = Size::new(crop.2, crop.3);
    }
    let pixels: Vec<u8> = match sample {
        SampleType::U8 => samples.iter().map(|&v| v as u8).collect(),
        _ => samples.iter().flat_map(|v| v.to_ne_bytes()).collect(),
    };
    let mut warnings = Vec::new();
    if avif.animated {
        warnings.push(ImportWarning::FirstFrameOnly);
    }
    // Unspecified code points: sRGB.
    let code = |v: u8, default: u8| if v == 2 { default } else { v };
    let space = if properties.icc.is_some() {
        None
    } else {
        cicp_code_space(code(primaries, 1), code(transfer, 13), 0, true)
    };
    if properties.icc.is_none() && space.is_none() {
        return Err(ImportError::UnsupportedPixels(format!(
            "AVIF color primaries {primaries} with transfer {transfer}"
        )));
    }
    Ok(Decoded {
        size,
        layout,
        sample,
        alpha: if avif.premultiplied {
            AlphaMode::Premultiplied
        } else {
            AlphaMode::Straight
        },
        icc: properties.icc.clone(),
        space,
        orientation: orientation(&properties.transforms),
        pixels,
        warnings,
    })
}

/// The size of an image item: a grid's output size, else its `ispe`.
fn image_size(item: &Item) -> Result<(u32, u32), String> {
    let size = if &item.kind == b"grid" {
        let (_, _, w, h) = container::grid_layout(&item.data)?;
        Some((w, h))
    } else {
        item.properties.size
    };
    size.filter(|&(w, h)| w > 0 && h > 0)
        .ok_or("AVIF image without a size".to_owned())
}

/// Decode `item` (an image, or a grid of `tiles`) and hand each picture to `place`, the target
/// positioned where it goes. `first` is the already decoded first picture, if any.
fn for_each_picture(
    item: &Item,
    tiles: &[Item],
    first: Option<Picture>,
    max_pixels: u32,
    target: &mut Target<'_>,
    place: &mut dyn FnMut(&Picture, &mut Target<'_>) -> Result<(), String>,
) -> Result<(), String> {
    let mut first = first;
    let mut picture_of = |item: &Item| match first.take() {
        Some(picture) => Ok(picture),
        None => ffi::decode(&item.data, max_pixels),
    };
    if tiles.is_empty() {
        let picture = picture_of(item)?;
        target.x0 = 0;
        target.y0 = 0;
        return place(&picture, target);
    }
    let (rows, columns, _, _) = container::grid_layout(&item.data)?;
    if tiles.len() != (rows * columns) as usize {
        return Err("AVIF grid with a wrong number of tiles".to_owned());
    }
    for (i, tile) in tiles.iter().enumerate() {
        let picture = picture_of(tile)?;
        // Every tile has the first one's size.
        let (r, c) = (i / columns as usize, i % columns as usize);
        target.x0 = c * picture.width as usize;
        target.y0 = r * picture.height as usize;
        place(&picture, target)?;
    }
    target.x0 = 0;
    target.y0 = 0;
    Ok(())
}

/// The clean aperture as (left, top, width, height), when it is whole pixels within the image
/// (otherwise it is ignored, as libavif does).
fn clean_aperture(p: &Properties, size: Size) -> Option<(u32, u32, u32, u32)> {
    let [(wn, wd), (hn, hd), (hon, hod), (von, vod)] = p.clean_aperture?;
    if wd <= 0 || hd <= 0 || hod <= 0 || vod <= 0 || wn % wd != 0 || hn % hd != 0 {
        return None;
    }
    let (cw, ch) = (wn / wd, hn / hd);
    let (w, h) = (i64::from(size.width), i64::from(size.height));
    // The center is the image's, moved by the offsets: in halves, so that it stays whole.
    let left2 = (w - 1) * hod + 2 * hon - (cw - 1) * hod;
    let top2 = (h - 1) * vod + 2 * von - (ch - 1) * vod;
    if left2 % (2 * hod) != 0 || top2 % (2 * vod) != 0 {
        return None;
    }
    let (left, top) = (left2 / (2 * hod), top2 / (2 * vod));
    let inside = cw > 0 && ch > 0 && left >= 0 && top >= 0 && left + cw <= w && top + ch <= h;
    if !inside || (cw, ch) == (w, h) {
        return None;
    }
    Some((left as u32, top as u32, cw as u32, ch as u32))
}

fn cropped(
    samples: &[u16],
    size: Size,
    channels: usize,
    (left, top, w, h): (u32, u32, u32, u32),
) -> Vec<u16> {
    let stride = size.width as usize * channels;
    let (left, top, w) = (left as usize, top as usize, w as usize);
    (top..top + h as usize)
        .flat_map(|y| {
            let start = y * stride + left * channels;
            samples[start..start + w * channels].iter().copied()
        })
        .collect()
}

/// The orientation that `irot` and `imir` make, in their order: a clockwise rotation (EXIF's
/// sense) of `turns` quarters, then a horizontal mirror if `mirrored`.
fn orientation(transforms: &[Transform]) -> Orientation {
    let (mut turns, mut mirrored) = (0u8, false);
    fn rotate_clockwise(q: u8, turns: &mut u8, mirrored: bool) {
        // After a mirror, a rotation goes the other way.
        *turns = if mirrored {
            (*turns + 4 - q % 4) % 4
        } else {
            (*turns + q) % 4
        };
    }
    for t in transforms {
        match *t {
            // Anticlockwise quarters.
            Transform::Rotate(a) => rotate_clockwise((4 - a % 4) % 4, &mut turns, mirrored),
            Transform::MirrorHorizontally => mirrored = !mirrored,
            // About the horizontal axis: a half turn, then a horizontal mirror.
            Transform::MirrorVertically => {
                rotate_clockwise(2, &mut turns, mirrored);
                mirrored = !mirrored;
            }
        }
    }
    match (turns, mirrored) {
        (0, false) => Orientation::Normal,
        (1, false) => Orientation::Rotate90,
        (2, false) => Orientation::Rotate180,
        (3, false) => Orientation::Rotate270,
        (0, true) => Orientation::FlipHorizontal,
        (1, true) => Orientation::Transpose,
        (2, true) => Orientation::FlipVertical,
        _ => Orientation::Transverse,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/avif")
            .join(name)
    }

    /// The mean absolute difference of 8-bit RGB pixels from `rgb8`'s pattern (see the
    /// fixtures' README).
    fn distance_from_pattern(pixels: &[u8]) -> f64 {
        let expected = (0..48u32).flat_map(|y| {
            (0..64u32).flat_map(move |x| {
                [
                    (x * 4 % 256) as u8,
                    (y * 5 % 256) as u8,
                    ((x + y) * 3 % 256) as u8,
                ]
            })
        });
        let total: f64 = pixels
            .iter()
            .zip(expected)
            .map(|(a, b)| f64::from(a.abs_diff(b)))
            .sum();
        total / pixels.len() as f64
    }

    #[test]
    fn eight_bit_files_decode_close_to_their_source_in_srgb() {
        for name in ["rgb8-lossy.avif", "rgb8-q100.avif"] {
            let d = decode(&fixture(name)).unwrap();
            assert_eq!(
                (d.layout, d.sample),
                (ChannelLayout::Rgb, SampleType::U8),
                "{name}"
            );
            assert_eq!(d.size, Size::new(64, 48));
            assert_eq!(d.space, Some(slopshop_core::color::ColorSpace::SRGB));
            let distance = distance_from_pattern(&d.pixels);
            assert!(distance < 4.0, "{name}: {distance}");
            assert!(d.warnings.is_empty());
        }
    }

    #[test]
    fn deep_files_with_alpha_come_back_as_16_bit_rgba() {
        let d = decode(&fixture("rgba12.avif")).unwrap();
        assert_eq!((d.layout, d.sample), (ChannelLayout::Rgba, SampleType::U16));
        assert_eq!(d.alpha, AlphaMode::Straight);
        let samples: Vec<u16> = d
            .pixels
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_ne_bytes(*b))
            .collect();
        // Alpha: `1000 + (x + y) · 500` within 1 %.
        for (i, px) in samples.as_chunks::<4>().0.iter().enumerate() {
            let (x, y) = ((i % 64) as f64, (i / 64) as f64);
            let expected = 1000.0 + (x + y) * 500.0;
            assert!(
                (f64::from(px[3]) - expected).abs() < 655.0,
                "alpha at {i}: {} vs {expected}",
                px[3]
            );
        }
    }

    #[test]
    fn rotations_orient_the_image() {
        let d = decode(&fixture("rotated.avif")).unwrap();
        assert_eq!(d.orientation, Orientation::Rotate90);
        let imported = crate::open_image(&fixture("rotated.avif")).unwrap();
        assert_eq!(imported.image.size(), Size::new(48, 64));
    }

    #[test]
    fn damaged_files_are_errors() {
        let bytes = std::fs::read(fixture("rgba12.avif")).unwrap();
        assert!(is_avif(&bytes));
        let path =
            std::env::temp_dir().join(format!("slopshop-avif-{}-cut.avif", std::process::id()));
        for len in [40, bytes.len() / 2, bytes.len() - 10] {
            std::fs::write(&path, &bytes[..len]).unwrap();
            assert!(decode(&path).is_err(), "{len}");
        }
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn rotations_and_mirrors_compose_into_exif_orientations() {
        use Transform::*;
        assert_eq!(orientation(&[]), Orientation::Normal);
        // A quarter turn anticlockwise is EXIF's "rotate 270° clockwise".
        assert_eq!(orientation(&[Rotate(1)]), Orientation::Rotate270);
        assert_eq!(orientation(&[Rotate(3)]), Orientation::Rotate90);
        assert_eq!(orientation(&[Rotate(2)]), Orientation::Rotate180);
        assert_eq!(
            orientation(&[MirrorHorizontally]),
            Orientation::FlipHorizontal
        );
        assert_eq!(orientation(&[MirrorVertically]), Orientation::FlipVertical);
        assert_eq!(
            orientation(&[MirrorHorizontally, MirrorVertically]),
            Orientation::Rotate180
        );
        // Clockwise quarter turn, then a horizontal mirror: EXIF 5 (transpose).
        assert_eq!(
            orientation(&[Rotate(3), MirrorHorizontally]),
            Orientation::Transpose
        );
    }

    #[test]
    fn clean_apertures_crop_whole_centered_areas() {
        let size = Size::new(100, 80);
        let with = |clean_aperture: [(i64, i64); 4]| Properties {
            clean_aperture: Some(clean_aperture),
            ..Properties::default()
        };
        // 60 × 40, centered.
        let centered = with([(60, 1), (40, 1), (0, 1), (0, 1)]);
        assert_eq!(clean_aperture(&centered, size), Some((20, 20, 60, 40)));
        // Moved 10 to the right.
        let moved = with([(60, 1), (40, 1), (10, 1), (0, 1)]);
        assert_eq!(clean_aperture(&moved, size), Some((30, 20, 60, 40)));
        // Half pixels, or outside: ignored.
        let half = with([(61, 1), (40, 1), (0, 1), (0, 1)]);
        assert_eq!(clean_aperture(&half, size), None);
        let outside = with([(60, 1), (40, 1), (40, 1), (0, 1)]);
        assert_eq!(clean_aperture(&outside, size), None);
    }
}
