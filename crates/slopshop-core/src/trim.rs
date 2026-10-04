//! Image > Trim, as Photoshop's: the canvas reduced to the composited image without its
//! uniform margins, fully transparent ones or those of the color of a corner pixel. Only the
//! margins are composited, in bands from each edge inward, never the whole image at once.

use crate::composite::{CompositeError, composite_region};
use crate::document::Document;
use crate::geom::Rect;

/// What a margin is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrimBasis {
    /// Fully transparent pixels.
    Transparent,
    /// Pixels of the color of the top-left one.
    TopLeftColor,
    /// Pixels of the color of the bottom-right one.
    BottomRightColor,
}

/// The sides that may be trimmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrimSides {
    pub top: bool,
    pub bottom: bool,
    pub left: bool,
    pub right: bool,
}

impl TrimSides {
    pub const ALL: TrimSides = TrimSides {
        top: true,
        bottom: true,
        left: true,
        right: true,
    };
}

/// Composited values per band (premultiplied RGBA `f32`): 4 Mi of them, 16 MiB.
const BAND_VALUES: u64 = 4 << 20;

/// The part of the canvas left once the margins of `basis` are taken off `sides`, `[x, y,
/// width, height]` in document pixels; `None` when there is nothing to trim, or when the
/// whole canvas is margin (as in Photoshop, nothing is done). Colors match exactly, as
/// composited (premultiplied, in the working space).
pub fn trim_area(
    document: &Document,
    basis: TrimBasis,
    sides: TrimSides,
) -> Result<Option<[i64; 4]>, CompositeError> {
    let size = document.size();
    let (width, height) = (size.width, size.height);
    if width == 0 || height == 0 {
        return Ok(None);
    }
    let reference = match basis {
        TrimBasis::Transparent => None,
        TrimBasis::TopLeftColor => Some(pixel(document, 0, 0)?),
        TrimBasis::BottomRightColor => Some(pixel(document, width - 1, height - 1)?),
    };
    let margin = |px: &[f32; 4]| match reference {
        None => px[3] <= 0.0,
        Some(color) => *px == color,
    };
    let row_is_margin = |y: u32, rows: &[f32]| {
        let pixels = rows.as_chunks::<4>().0;
        pixels[y as usize * width as usize..][..width as usize]
            .iter()
            .all(margin)
    };

    // Rows from the top, then from the bottom.
    let band_rows = band(width);
    let mut top = 0;
    if sides.top {
        top = scan(height, band_rows, false, |from, count| {
            let rows = composite(document, Rect::new(0, from, width, count))?;
            Ok((0..count).map(|y| row_is_margin(y, &rows)).collect())
        })?;
        if top == height {
            return Ok(None);
        }
    }
    let mut bottom = height;
    if sides.bottom {
        let kept = height - top;
        let trimmed = scan(kept, band_rows, true, |from, count| {
            let rows = composite(document, Rect::new(0, top + from, width, count))?;
            Ok((0..count).map(|y| row_is_margin(y, &rows)).collect())
        })?;
        if trimmed == kept {
            return Ok(None);
        }
        bottom = height - trimmed;
    }

    // Columns between them, from the left, then from the right.
    let rows = bottom - top;
    let band_columns = band(rows);
    let column_is_margin = |x: u32, count: u32, values: &[f32]| {
        let pixels = values.as_chunks::<4>().0;
        (0..rows as usize).all(|y| margin(&pixels[y * count as usize + x as usize]))
    };
    let mut left = 0;
    if sides.left {
        left = scan(width, band_columns, false, |from, count| {
            let values = composite(document, Rect::new(from, top, count, rows))?;
            Ok((0..count)
                .map(|x| column_is_margin(x, count, &values))
                .collect())
        })?;
        if left == width {
            return Ok(None);
        }
    }
    let mut right = width;
    if sides.right {
        let kept = width - left;
        let trimmed = scan(kept, band_columns, true, |from, count| {
            let values = composite(document, Rect::new(left + from, top, count, rows))?;
            Ok((0..count)
                .map(|x| column_is_margin(x, count, &values))
                .collect())
        })?;
        right = width - trimmed;
    }

    if (left, top, right, bottom) == (0, 0, width, height) {
        return Ok(None);
    }
    Ok(Some([
        i64::from(left),
        i64::from(top),
        i64::from(right - left),
        i64::from(bottom - top),
    ]))
}

/// How many lines of `length` pixels a band holds.
fn band(length: u32) -> u32 {
    (BAND_VALUES / (u64::from(length) * 4)).clamp(1, u64::from(u32::MAX)) as u32
}

/// How many of `count` lines are margin from one end (`from_end`: the last ones), asking
/// `margins(first, n)` whether lines `first..first + n` are, `band` lines at a time.
fn scan(
    count: u32,
    band: u32,
    from_end: bool,
    mut margins: impl FnMut(u32, u32) -> Result<Vec<bool>, CompositeError>,
) -> Result<u32, CompositeError> {
    let mut done = 0;
    while done < count {
        let n = band.min(count - done);
        let first = if from_end { count - done - n } else { done };
        let lines = margins(first, n)?;
        let run = if from_end {
            lines.iter().rev().take_while(|&&m| m).count()
        } else {
            lines.iter().take_while(|&&m| m).count()
        } as u32;
        done += run;
        if run < n {
            break;
        }
    }
    Ok(done)
}

fn composite(document: &Document, region: Rect) -> Result<Vec<f32>, CompositeError> {
    let mut out = vec![0.0; region.size().pixel_count() as usize * 4];
    composite_region(document, region, &mut out)?;
    Ok(out)
}

fn pixel(document: &Document, x: u32, y: u32) -> Result<[f32; 4], CompositeError> {
    let values = composite(document, Rect::new(x, y, 1, 1))?;
    Ok([values[0], values[1], values[2], values[3]])
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::blend::BlendMode;
    use crate::color::{LinearRgba, PixelFormat};
    use crate::document::{Layer, LayerContent};
    use crate::edit::Edit;
    use crate::geom::Size;
    use crate::raster::RasterImage;
    use crate::transform::Affine;

    fn push(doc: &mut Document, content: LayerContent) {
        let layer = Layer {
            style: None,
            id: doc.allocate_layer_id(),
            name: "l".into(),
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask: None,
            clipped: false,
            transform: Affine::IDENTITY,
            content,
        };
        let index = doc.layers().len();
        Edit::InsertLayer {
            parent: None,
            index,
            layer,
        }
        .apply(doc)
        .unwrap();
    }

    /// An opaque red `rect` on a transparent layer of `size`.
    fn square(size: Size, rect: Rect) -> LayerContent {
        let pixels = [200u8, 10, 10, 255].repeat(rect.size().pixel_count() as usize);
        let image = RasterImage::from_placed(size, PixelFormat::RGBA8_SRGB, rect, &pixels, &[0; 4])
            .unwrap();
        LayerContent::raster(Arc::new(image))
    }

    #[test]
    fn transparent_margins_are_trimmed_on_the_sides_asked() {
        let size = Size::new(30, 20);
        let mut doc = Document::new(size);
        push(&mut doc, square(size, Rect::new(4, 3, 10, 6)));
        assert_eq!(
            trim_area(&doc, TrimBasis::Transparent, TrimSides::ALL),
            Ok(Some([4, 3, 10, 6]))
        );
        let sides = TrimSides {
            top: false,
            right: false,
            ..TrimSides::ALL
        };
        assert_eq!(
            trim_area(&doc, TrimBasis::Transparent, sides),
            Ok(Some([4, 0, 26, 9]))
        );
        // An opaque background: no transparent margin.
        let mut opaque = doc.clone();
        Edit::InsertLayer {
            parent: None,
            index: 0,
            layer: Layer {
                style: None,
                id: opaque.allocate_layer_id(),
                name: "fill".into(),
                visible: true,
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                mask: None,
                clipped: false,
                transform: Affine::IDENTITY,
                content: LayerContent::Fill {
                    color: LinearRgba::new(1.0, 1.0, 1.0, 1.0),
                },
            },
        }
        .apply(&mut opaque)
        .unwrap();
        assert_eq!(
            trim_area(&opaque, TrimBasis::Transparent, TrimSides::ALL),
            Ok(None)
        );
        // The corner's color: the white around the square.
        assert_eq!(
            trim_area(&opaque, TrimBasis::TopLeftColor, TrimSides::ALL),
            Ok(Some([4, 3, 10, 6]))
        );
        assert_eq!(
            trim_area(&opaque, TrimBasis::BottomRightColor, TrimSides::ALL),
            Ok(Some([4, 3, 10, 6]))
        );
    }

    #[test]
    fn a_canvas_all_margin_or_without_margin_is_left_alone() {
        let size = Size::new(8, 8);
        let mut doc = Document::new(size);
        assert_eq!(
            trim_area(&doc, TrimBasis::Transparent, TrimSides::ALL),
            Ok(None)
        );
        push(&mut doc, square(size, Rect::new(0, 0, 8, 8)));
        assert_eq!(
            trim_area(&doc, TrimBasis::Transparent, TrimSides::ALL),
            Ok(None)
        );
        assert_eq!(
            trim_area(&doc, TrimBasis::TopLeftColor, TrimSides::ALL),
            Ok(None)
        );
    }

    #[test]
    fn bands_find_the_same_margins_as_one_line_at_a_time() {
        // Margins across several bands, from both ends.
        let lines: Vec<bool> = (0..100).map(|i| !(37..=80).contains(&i)).collect();
        for band in [1, 3, 7, 100, 1000] {
            let ask = |first: u32, n: u32| Ok(lines[first as usize..][..n as usize].to_vec());
            assert_eq!(scan(100, band, false, ask), Ok(37), "{band}");
            assert_eq!(scan(100, band, true, ask), Ok(19), "{band}");
        }
        let all = |_: u32, n: u32| Ok(vec![true; n as usize]);
        assert_eq!(scan(10, 3, false, all), Ok(10));
    }
}
