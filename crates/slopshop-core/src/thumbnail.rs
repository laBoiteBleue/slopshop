//! Small previews of raster images, for UI thumbnails (layers panel).
//!
//! A thumbnail is read from the coarsest pyramid level that is still at least as large as the
//! thumbnail (a few thousand texels at most, whatever the image size), box-averaged in linear
//! light with premultiplied alpha, then converted like an 8-bit sRGB export: straight alpha,
//! exact quantization, no dither. It is a display view: the image is never changed.

use crate::blend::BlendSpace;
use crate::color::{PixelFormat, WORKING_SPACE, mat_vec};
use crate::convert::{ConversionReport, ConvertOptions, Converter, WHITE_MATTE};
use crate::geom::Size;
use crate::raster::{Codec, RasterImage, TILE_SIZE};
use crate::tile::TileCoord;

/// An RGBA8 sRGB preview, straight alpha, row-major.
#[derive(Debug, Clone, PartialEq)]
pub struct Thumbnail {
    pub size: Size,
    pub pixels: Vec<u8>,
}

/// `image` fitted in `max_side`×`max_side` pixels, keeping its aspect ratio; never enlarged.
/// `max_side` 0 is treated as 1.
pub fn raster_thumbnail(image: &RasterImage, max_side: u32) -> Thumbnail {
    thumbnail_as(image, max_side, PixelFormat::RGBA8_SRGB)
}

/// A mask's thumbnail (ADR 0014): its coverage values shown as they are (50% coverage is code
/// 128, mid gray, as in Photoshop), not as light encoded for display.
pub fn mask_thumbnail(mask: &RasterImage, max_side: u32) -> Thumbnail {
    let raw = PixelFormat {
        color_space: crate::color::ColorSpace::LINEAR_SRGB,
        ..PixelFormat::RGBA8_SRGB
    };
    thumbnail_as(mask, max_side, raw)
}

fn thumbnail_as(image: &RasterImage, max_side: u32, target: PixelFormat) -> Thumbnail {
    let max_side = max_side.max(1);
    let full = image.size();
    let longest = full.width.max(full.height);
    let scale = (f64::from(max_side) / f64::from(longest)).min(1.0);
    let size = Size::new(
        ((f64::from(full.width) * scale).round() as u32).max(1),
        ((f64::from(full.height) * scale).round() as u32).max(1),
    );

    // The coarsest level at least as large as the thumbnail in both directions.
    let levels = image.levels();
    let level = levels
        .iter()
        .rev()
        .find(|l| l.size().width >= size.width && l.size().height >= size.height)
        .unwrap_or(&levels[0]);
    let source = level.size();
    let codec = Codec::new(image.stored_format());
    let matrix = image.matrix_to(&WORKING_SPACE);

    let texel = |x: u32, y: u32| -> [f64; 4] {
        let coord = TileCoord {
            col: x / TILE_SIZE,
            row: y / TILE_SIZE,
        };
        let Some(tile) = level.tile(coord) else {
            return [0.0; 4];
        };
        let offset = ((y % TILE_SIZE) * TILE_SIZE + x % TILE_SIZE) as usize * codec.bytes_per_pixel;
        let Some(px) = tile.get(offset..offset + codec.bytes_per_pixel) else {
            return [0.0; 4];
        };
        // Display values: non-finite samples read as 0, like the viewport clamps them.
        let (color, alpha) = codec.read_mapped(px, &mut |v| if v.is_finite() { v } else { 0.0 });
        let [r, g, b] = mat_vec(&matrix, color.map(f64::from));
        [r, g, b, f64::from(alpha)]
    };

    // Box filter: every output pixel averages the source texels its footprint touches.
    let span = |i: u32, out: u32, src: u32| {
        let lo = (u64::from(i) * u64::from(src) / u64::from(out)) as u32;
        let hi = ((u64::from(i) + 1) * u64::from(src)).div_ceil(u64::from(out)) as u32;
        (lo, hi.max(lo + 1).min(src))
    };
    let mut linear = Vec::with_capacity(size.pixel_count() as usize * 4);
    for ty in 0..size.height {
        let (y0, y1) = span(ty, size.height, source.height);
        for tx in 0..size.width {
            let (x0, x1) = span(tx, size.width, source.width);
            let mut sum = [0.0f64; 4];
            for y in y0..y1 {
                for x in x0..x1 {
                    for (s, v) in sum.iter_mut().zip(texel(x, y)) {
                        *s += v;
                    }
                }
            }
            let n = f64::from((x1 - x0) * (y1 - y0));
            linear.extend(sum.map(|s| (s / n) as f32));
        }
    }

    let converter = Converter::new(
        target,
        ConvertOptions {
            dither: false,
            big_endian: false,
            matte: WHITE_MATTE,
            blend_space: BlendSpace::Linear,
        },
    );
    let mut pixels = vec![0u8; size.pixel_count() as usize * 4];
    if let Ok(converter) = converter {
        let row = size.width as usize * 4;
        let mut report = ConversionReport::default();
        for (y, (src, dst)) in linear
            .chunks_exact(row)
            .zip(pixels.chunks_exact_mut(row))
            .enumerate()
        {
            // Invariant: rows of the right length, so conversion cannot fail.
            let _ = converter.convert_row(src, 0, y as u32, dst, &mut report);
        }
    }
    Thumbnail { size, pixels }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{AlphaMode, ChannelLayout, ColorSpace, SampleType};

    fn rgba8(size: Size, pixel: impl Fn(u32, u32) -> [u8; 4]) -> RasterImage {
        let mut pixels = Vec::with_capacity(size.pixel_count() as usize * 4);
        for y in 0..size.height {
            for x in 0..size.width {
                pixels.extend(pixel(x, y));
            }
        }
        RasterImage::from_pixels(size, PixelFormat::RGBA8_SRGB, &pixels).unwrap()
    }

    #[test]
    fn keeps_the_aspect_ratio_and_never_enlarges() {
        let wide = RasterImage::from_pixels(
            Size::new(2000, 500),
            PixelFormat::RGBA8_SRGB,
            &vec![255; 2000 * 500 * 4],
        )
        .unwrap();
        let t = raster_thumbnail(&wide, 64);
        assert_eq!(t.size, Size::new(64, 16));
        assert_eq!(t.pixels.len(), 64 * 16 * 4);
        assert!(t.pixels.iter().all(|&v| v == 255), "white stays white");

        let small = RasterImage::from_pixels(
            Size::new(10, 3),
            PixelFormat::RGBA8_SRGB,
            &[128; 10 * 3 * 4],
        )
        .unwrap();
        assert_eq!(raster_thumbnail(&small, 64).size, Size::new(10, 3));
        let tall = RasterImage::from_pixels(
            Size::new(3, 900),
            PixelFormat::RGBA8_SRGB,
            &vec![0; 3 * 900 * 4],
        )
        .unwrap();
        assert_eq!(raster_thumbnail(&tall, 64).size, Size::new(1, 64));
    }

    #[test]
    fn an_unscaled_thumbnail_is_the_image_itself() {
        // 8-bit sRGB in, 8-bit sRGB out, same size: the exact pixels (alpha 0 excepted).
        let size = Size::new(20, 7);
        let pattern = |x: u32, y: u32| [(x * 12) as u8, (y * 30) as u8, (x ^ y) as u8, 255];
        let image = rgba8(size, pattern);
        let t = raster_thumbnail(&image, 64);
        let expected: Vec<u8> = (0..7)
            .flat_map(|y| (0..20).flat_map(move |x| pattern(x, y)))
            .collect();
        assert_eq!(t.pixels, expected);
    }

    #[test]
    fn averages_in_linear_light_and_reads_gray_and_float_images() {
        // Black and white stripes one texel wide average to linear 0.5 (sRGB code 188).
        let size = Size::new(512, 512);
        let stripes = rgba8(size, |x, _| {
            let v = if x % 2 == 0 { 0 } else { 255 };
            [v, v, v, 255]
        });
        let t = raster_thumbnail(&stripes, 64);
        assert!(
            t.pixels
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| p[0].abs_diff(188) <= 1 && p[3] == 255)
        );

        let gray = PixelFormat {
            layout: ChannelLayout::GrayAlpha,
            sample: SampleType::F32,
            color_space: ColorSpace::LINEAR_SRGB,
            alpha: AlphaMode::Straight,
        };
        let px: Vec<u8> = [0.5f32, 0.5].iter().flat_map(|v| v.to_ne_bytes()).collect();
        let image = RasterImage::from_pixels(Size::new(1, 1), gray, &px).unwrap();
        let t = raster_thumbnail(&image, 16);
        assert_eq!(t.size, Size::new(1, 1));
        // Linear 0.5 gray at half alpha: sRGB 188, straight.
        assert!(
            t.pixels[0].abs_diff(188) <= 1 && t.pixels[3].abs_diff(128) <= 1,
            "{:?}",
            t.pixels
        );
    }

    #[test]
    fn mask_thumbnails_show_coverage_values_as_they_are() {
        let gray = PixelFormat {
            layout: ChannelLayout::Gray,
            sample: SampleType::U8,
            color_space: ColorSpace::LINEAR_SRGB,
            alpha: AlphaMode::Straight,
        };
        let mask = RasterImage::from_pixels(Size::new(2, 1), gray, &[128, 255]).unwrap();
        let t = mask_thumbnail(&mask, 16);
        assert_eq!(t.pixels, [128, 128, 128, 255, 255, 255, 255, 255]);
    }
}
