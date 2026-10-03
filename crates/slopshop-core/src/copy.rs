//! The clipboard's pixels and places: what Edit > Copy Merged takes from a document, the image
//! other applications get from a copy, and where Edit > Paste puts what it pastes.

use crate::blend::BlendSpace;
use crate::color::{AlphaMode, ChannelLayout, PixelFormat, SampleType, WORKING_SPACE};
use crate::convert::{ConversionReport, ConvertError, ConvertOptions, Converter, WHITE_MATTE};
use crate::geom::{Rect, Size};
use crate::paint::MaskReader;
use crate::raster::{Codec, RasterError, RasterImage};
use crate::selection::Selection;

/// The pixels of a Copy Merged: the working space in half floats, premultiplied. It holds every
/// value the display shows (half floats are the display bound, ADR 0007) at half the memory of
/// the compositors' `f32`, and no color is converted.
pub const MERGED_FORMAT: PixelFormat = PixelFormat {
    layout: ChannelLayout::Rgba,
    sample: SampleType::F16,
    color_space: WORKING_SPACE,
    alpha: AlphaMode::Premultiplied,
};

#[derive(Debug)]
pub enum CopyError {
    Convert(ConvertError),
    Raster(RasterError),
    /// Pixels and size disagree.
    BufferSizeMismatch,
}

impl std::fmt::Display for CopyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CopyError::Convert(e) => write!(f, "{e:?}"),
            CopyError::Raster(e) => write!(f, "{e}"),
            CopyError::BufferSizeMismatch => write!(f, "pixels and size disagree"),
        }
    }
}

impl std::error::Error for CopyError {}

impl From<ConvertError> for CopyError {
    fn from(e: ConvertError) -> Self {
        CopyError::Convert(e)
    }
}

impl From<RasterError> for CopyError {
    fn from(e: RasterError) -> Self {
        CopyError::Raster(e)
    }
}

/// Composited pixels of `region` (premultiplied working-space RGBA `f32`, as the compositors
/// give them) kept where `selection` selects them, in part where it is soft.
pub fn limit_to_selection(pixels: &mut [f32], region: Rect, selection: &Selection) {
    let image = selection.image();
    let codec = Codec::new(image.stored_format());
    let reader = MaskReader {
        image,
        codec: &codec,
    };
    let width = region.width as usize;
    for (row, line) in pixels.chunks_exact_mut(width * 4).enumerate() {
        let y = f64::from(region.y) + row as f64;
        for (column, px) in line.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let coverage = reader.at(f64::from(region.x) + column as f64, y);
            if coverage < 1.0 {
                px.iter_mut().for_each(|v| *v *= coverage);
            }
        }
    }
}

/// Composited rows converted for the clipboard, band by band (so that a large copy never needs
/// all of its `f32` pixels at once).
#[derive(Debug)]
pub struct Rows {
    converter: Converter,
    width: usize,
}

impl Rows {
    /// Rows of a Copy Merged, in [`MERGED_FORMAT`].
    pub fn merged(width: u32, blend_space: BlendSpace) -> Result<Self, CopyError> {
        Self::new(width, MERGED_FORMAT, blend_space, false)
    }

    /// Rows of the image other applications get from a copy: 8-bit sRGB with straight alpha,
    /// what system clipboards hold. An explicit, lossy conversion (dithered like an 8-bit
    /// export); SlopShop pastes its own copy instead.
    pub fn standard(width: u32, blend_space: BlendSpace) -> Result<Self, CopyError> {
        Self::new(width, PixelFormat::RGBA8_SRGB, blend_space, true)
    }

    fn new(
        width: u32,
        target: PixelFormat,
        blend_space: BlendSpace,
        dither: bool,
    ) -> Result<Self, CopyError> {
        let options = ConvertOptions {
            dither,
            big_endian: cfg!(target_endian = "big"),
            matte: WHITE_MATTE,
            blend_space,
        };
        Ok(Self {
            converter: Converter::new(target, options)?,
            width: width as usize,
        })
    }

    /// Append `pixels` (whole rows of premultiplied working-space RGBA `f32`, the first one being
    /// row `first_row` of the copy) to `out`, converted.
    pub fn convert(
        &self,
        pixels: &[f32],
        first_row: u32,
        out: &mut Vec<u8>,
    ) -> Result<(), CopyError> {
        let (width, bpp) = (self.width, self.converter.bytes_per_pixel());
        if width == 0 || !pixels.len().is_multiple_of(width * 4) {
            return Err(CopyError::BufferSizeMismatch);
        }
        let start = out.len();
        out.resize(start + pixels.len() / 4 * bpp, 0);
        // What is lost is the point of the target: nothing is reported.
        let mut report = ConversionReport::default();
        for (y, (src, dst)) in pixels
            .chunks_exact(width * 4)
            .zip(out[start..].chunks_exact_mut(width * bpp))
            .enumerate()
        {
            self.converter
                .convert_row(src, 0, first_row + y as u32, dst, &mut report)?;
        }
        Ok(())
    }
}

/// A Copy Merged of `pixels` (premultiplied working-space RGBA `f32`, `size`): a raster in
/// [`MERGED_FORMAT`].
pub fn merged_image(
    pixels: &[f32],
    size: Size,
    blend_space: BlendSpace,
) -> Result<RasterImage, CopyError> {
    let mut bytes = Vec::new();
    Rows::merged(size.width, blend_space)?.convert(pixels, 0, &mut bytes)?;
    Ok(RasterImage::from_pixels(size, MERGED_FORMAT, &bytes)?)
}

/// A fingerprint of an 8-bit RGBA image (`size`, straight alpha) that survives what system
/// clipboards do to it: its size, its alpha and the color of its opaque pixels (macOS keeps
/// premultiplied colors, which changes the others). Paste compares it to tell SlopShop's own
/// copy from an image copied elsewhere since.
pub fn fingerprint(size: Size, rgba: &[u8]) -> u64 {
    // FNV-1a, 64 bits.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut add = |byte: u8| {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    };
    for byte in size
        .width
        .to_le_bytes()
        .into_iter()
        .chain(size.height.to_le_bytes())
    {
        add(byte);
    }
    for px in rgba.as_chunks::<4>().0 {
        add(px[3]);
        if px[3] == u8::MAX {
            add(px[0]);
            add(px[1]);
            add(px[2]);
        }
    }
    hash
}

/// How a paste places what it pastes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PasteMode {
    /// Edit > Paste: where it was copied when that is in sight, else in the middle of the view.
    Paste,
    /// Edit > Paste in Place: where it was copied, always.
    InPlace,
    /// Edit > Paste Into: where it was copied when that meets the selection (its bounds), else
    /// in the middle of it.
    Into { selection: [f64; 4] },
    /// Paste Here (the image's right-click menu): centered on the point clicked.
    At { point: [f64; 2] },
}

/// The whole-pixel offset (dx, dy) that places pasted content, as Photoshop does. `bounds`:
/// where it lies in the document, `[x0, y0, x1, y1)`; `placed`: it has a place (copied in
/// SlopShop), else it is an image from another application, `bounds` its size at the origin.
/// `view`: the part of the document shown, if known.
pub fn paste_offset(
    bounds: [f64; 4],
    placed: bool,
    mode: PasteMode,
    view: Option<[f64; 4]>,
    canvas: Size,
) -> (i64, i64) {
    let center = |[x0, y0, x1, y1]: [f64; 4]| ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    if let PasteMode::At { point: [px, py] } = mode {
        let (bx, by) = center(bounds);
        return ((px - bx).round() as i64, (py - by).round() as i64);
    }
    let canvas = [0.0, 0.0, f64::from(canvas.width), f64::from(canvas.height)];
    let area = match mode {
        PasteMode::Into { selection } => selection,
        PasteMode::At { .. } => canvas,
        PasteMode::Paste | PasteMode::InPlace => view
            .and_then(|view| intersection(view, canvas))
            .unwrap_or(canvas),
    };
    if placed && (mode == PasteMode::InPlace || intersection(bounds, area).is_some()) {
        return (0, 0);
    }
    let ((ax, ay), (bx, by)) = (center(area), center(bounds));
    ((ax - bx).round() as i64, (ay - by).round() as i64)
}

/// The overlap of two rectangles `[x0, y0, x1, y1)`, `None` when they do not overlap.
fn intersection(a: [f64; 4], b: [f64; 4]) -> Option<[f64; 4]> {
    let r = [
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    ];
    (r[0] < r[2] && r[1] < r[3]).then_some(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::{Combine, EdgeOptions, Shape, select_shape};
    use std::sync::Arc;

    const CANVAS: Size = Size::new(1000, 800);

    #[test]
    fn a_paste_stays_where_it_was_copied_when_in_sight() {
        let view = Some([100.0, 100.0, 500.0, 400.0]);
        let bounds = [450.0, 350.0, 600.0, 450.0];
        assert_eq!(
            paste_offset(bounds, true, PasteMode::Paste, view, CANVAS),
            (0, 0)
        );
    }

    #[test]
    fn a_paste_out_of_sight_lands_in_the_middle_of_the_view() {
        let view = Some([100.0, 100.0, 500.0, 400.0]);
        let bounds = [700.0, 500.0, 800.0, 600.0];
        // Center (750, 550) → the view's (300, 250).
        assert_eq!(
            paste_offset(bounds, true, PasteMode::Paste, view, CANVAS),
            (-450, -300)
        );
        // In place: where it was, always.
        assert_eq!(
            paste_offset(bounds, true, PasteMode::InPlace, view, CANVAS),
            (0, 0)
        );
    }

    #[test]
    fn an_image_from_elsewhere_is_centered_in_the_visible_canvas() {
        // The view shows beyond the canvas: only its visible part counts.
        let view = Some([-500.0, -400.0, 500.0, 400.0]);
        let image = [0.0, 0.0, 200.0, 100.0];
        assert_eq!(
            paste_offset(image, false, PasteMode::Paste, view, CANVAS),
            (150, 150)
        );
        assert_eq!(
            paste_offset(image, false, PasteMode::InPlace, view, CANVAS),
            (150, 150)
        );
        // Without a view, the canvas.
        assert_eq!(
            paste_offset(image, false, PasteMode::Paste, None, CANVAS),
            (400, 350)
        );
    }

    #[test]
    fn a_paste_into_a_selection_is_centered_on_it_unless_it_meets_it() {
        let into = PasteMode::Into {
            selection: [600.0, 600.0, 700.0, 700.0],
        };
        let bounds = [0.0, 0.0, 100.0, 100.0];
        assert_eq!(paste_offset(bounds, true, into, None, CANVAS), (600, 600));
        let meeting = [650.0, 650.0, 750.0, 750.0];
        assert_eq!(paste_offset(meeting, true, into, None, CANVAS), (0, 0));
    }

    #[test]
    fn paste_here_centers_on_the_point_whatever_was_copied() {
        let at = PasteMode::At {
            point: [300.0, 200.0],
        };
        let bounds = [0.0, 0.0, 100.0, 50.0];
        assert_eq!(paste_offset(bounds, true, at, None, CANVAS), (250, 175));
        assert_eq!(paste_offset(bounds, false, at, None, CANVAS), (250, 175));
    }

    #[test]
    fn copy_merged_keeps_the_selected_part_with_its_soft_edge() {
        let canvas = Size::new(4, 1);
        let shape = Shape::Rectangle {
            left: 1.0,
            top: 0.0,
            right: 2.5,
            bottom: 1.0,
        };
        let selection = select_shape(
            canvas,
            None,
            &shape,
            EdgeOptions::default(),
            Combine::Replace,
        )
        .unwrap()
        .unwrap();
        let selection = Selection::new(Arc::new(selection)).unwrap();
        let mut pixels = [0.5f32, 0.25, 1.0, 1.0].repeat(4);
        limit_to_selection(&mut pixels, Rect::new(0, 0, 4, 1), &selection);
        assert_eq!(&pixels[0..4], &[0.0; 4]);
        assert_eq!(&pixels[4..8], &[0.5, 0.25, 1.0, 1.0]);
        assert!((pixels[11] - 0.5).abs() < 1e-4);
        assert_eq!(&pixels[12..16], &[0.0; 4]);
        let image = merged_image(&pixels, canvas, BlendSpace::Perceptual).unwrap();
        assert_eq!(image.format(), MERGED_FORMAT);
        assert_eq!(image.alpha_at(0, 0), 0.0);
        assert_eq!(image.alpha_at(1, 0), 1.0);
        assert!((image.alpha_at(2, 0) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn the_standard_image_is_8_bit_srgb() {
        // Opaque linear 0.5 gray is sRGB 188; half transparent black stays black.
        let pixels = [0.5f32, 0.5, 0.5, 1.0, 0.0, 0.0, 0.0, 0.5];
        let mut bytes = Vec::new();
        Rows::standard(2, BlendSpace::Linear)
            .unwrap()
            .convert(&pixels, 0, &mut bytes)
            .unwrap();
        assert_eq!(bytes[3], 255);
        assert!((i32::from(bytes[0]) - 188).abs() <= 1, "{bytes:?}");
        assert_eq!(&bytes[4..7], &[0, 0, 0]);
        assert!((i32::from(bytes[7]) - 128).abs() <= 1, "{bytes:?}");
    }

    #[test]
    fn fingerprints_ignore_the_color_of_transparent_pixels() {
        let size = Size::new(2, 1);
        let a = [10, 20, 30, 255, 40, 50, 60, 128];
        let premultiplied = [10, 20, 30, 255, 20, 25, 30, 128];
        assert_eq!(fingerprint(size, &a), fingerprint(size, &premultiplied));
        let other = [11, 20, 30, 255, 40, 50, 60, 128];
        assert_ne!(fingerprint(size, &a), fingerprint(size, &other));
        assert_ne!(fingerprint(size, &a), fingerprint(Size::new(1, 2), &a));
    }
}
