//! EXIF orientation: lossless rotations and flips of packed pixel rows, for any pixel size.

use slopshop_core::Size;

/// The eight EXIF orientations (TIFF tag 274 values 1–8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Orientation {
    Normal,
    FlipHorizontal,
    Rotate180,
    FlipVertical,
    /// Transpose (mirror along the top-left/bottom-right diagonal).
    Transpose,
    Rotate90,
    Transverse,
    Rotate270,
}

impl Orientation {
    /// From an EXIF/TIFF orientation value; unknown values mean "as stored".
    pub fn from_exif(value: u16) -> Self {
        match value {
            2 => Self::FlipHorizontal,
            3 => Self::Rotate180,
            4 => Self::FlipVertical,
            5 => Self::Transpose,
            6 => Self::Rotate90,
            7 => Self::Transverse,
            8 => Self::Rotate270,
            _ => Self::Normal,
        }
    }

    pub fn from_image(orientation: image::metadata::Orientation) -> Self {
        use image::metadata::Orientation as O;
        match orientation {
            O::NoTransforms => Self::Normal,
            O::FlipHorizontal => Self::FlipHorizontal,
            O::Rotate180 => Self::Rotate180,
            O::FlipVertical => Self::FlipVertical,
            O::Rotate90FlipH => Self::Transpose,
            O::Rotate90 => Self::Rotate90,
            O::Rotate270FlipH => Self::Transverse,
            O::Rotate270 => Self::Rotate270,
        }
    }

    fn swaps_axes(self) -> bool {
        matches!(
            self,
            Self::Transpose | Self::Rotate90 | Self::Transverse | Self::Rotate270
        )
    }
}

/// Apply `orientation` to packed rows of `bpp`-byte pixels. `Normal` returns the input as is.
pub(crate) fn apply(
    pixels: Vec<u8>,
    size: Size,
    bpp: usize,
    orientation: Orientation,
) -> (Vec<u8>, Size) {
    if orientation == Orientation::Normal {
        return (pixels, size);
    }
    let (w, h) = (size.width as usize, size.height as usize);
    let out_size = if orientation.swaps_axes() {
        Size::new(size.height, size.width)
    } else {
        size
    };
    let ow = out_size.width as usize;
    let mut out = vec![0u8; pixels.len()];
    for y in 0..h {
        for x in 0..w {
            // Where the stored pixel (x, y) is displayed.
            let (dx, dy) = match orientation {
                Orientation::Normal => (x, y),
                Orientation::FlipHorizontal => (w - 1 - x, y),
                Orientation::Rotate180 => (w - 1 - x, h - 1 - y),
                Orientation::FlipVertical => (x, h - 1 - y),
                Orientation::Transpose => (y, x),
                Orientation::Rotate90 => (h - 1 - y, x),
                Orientation::Transverse => (h - 1 - y, w - 1 - x),
                Orientation::Rotate270 => (y, w - 1 - x),
            };
            let src = (y * w + x) * bpp;
            let dst = (dy * ow + dx) * bpp;
            out[dst..dst + bpp].copy_from_slice(&pixels[src..src + bpp]);
        }
    }
    (out, out_size)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 3×2 image of one-byte pixels:
    /// 1 2 3
    /// 4 5 6
    fn sample() -> (Vec<u8>, Size) {
        (vec![1, 2, 3, 4, 5, 6], Size::new(3, 2))
    }

    #[test]
    fn all_orientations() {
        let cases = [
            (1, vec![1, 2, 3, 4, 5, 6], Size::new(3, 2)),
            (2, vec![3, 2, 1, 6, 5, 4], Size::new(3, 2)),
            (3, vec![6, 5, 4, 3, 2, 1], Size::new(3, 2)),
            (4, vec![4, 5, 6, 1, 2, 3], Size::new(3, 2)),
            (5, vec![1, 4, 2, 5, 3, 6], Size::new(2, 3)),
            // Rotate 90° clockwise to display: the left column becomes the top row, reversed.
            (6, vec![4, 1, 5, 2, 6, 3], Size::new(2, 3)),
            (7, vec![6, 3, 5, 2, 4, 1], Size::new(2, 3)),
            (8, vec![3, 6, 2, 5, 1, 4], Size::new(2, 3)),
        ];
        for (exif, expected, size) in cases {
            let (px, s) = sample();
            let (out, out_size) = apply(px, s, 1, Orientation::from_exif(exif));
            assert_eq!((out, out_size), (expected, size), "EXIF {exif}");
        }
    }

    #[test]
    fn multi_byte_pixels_move_as_a_whole() {
        let px: Vec<u8> = (0u8..8).collect(); // 2×1 image of 4-byte pixels
        let (out, size) = apply(px, Size::new(2, 1), 4, Orientation::FlipHorizontal);
        assert_eq!(out, [4, 5, 6, 7, 0, 1, 2, 3]);
        assert_eq!(size, Size::new(2, 1));
    }
}
