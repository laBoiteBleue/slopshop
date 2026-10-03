//! Moving selected pixels: the Move tool dragged from inside a selection, as in Photoshop. The
//! selected pixels of an image leave a hole (transparent; hidden in a mask) and land shifted,
//! over what is there; Alt copies them instead (no hole). Soft selections move their pixels in
//! part, as Photoshop does.
//!
//! The result is a new image sharing every tile the move does not reach, computed from the
//! image the move started from, so moving again from there never compounds. It becomes the
//! layer's painted image (ADR 0027): the layer's original stays intact, and Delete Paint brings
//! it back.

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::blend::{BlendMode, BlendSpace, Blender};
use crate::color::{ChannelLayout, IDENTITY, Mat3, WORKING_SPACE, mat_vec};
use crate::paint::{MaskReader, PaintError};
use crate::raster::{Codec, RasterImage, TILE_SIZE, pad_tile, parallel_for_each};
use crate::selection::{SELECTION_FORMAT, Selection};
use crate::tile::TileCoord;
use crate::transform::Affine;

const T: usize = TILE_SIZE as usize;

/// What a move does to the pixels it takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveMode {
    /// They leave a hole (the Move tool).
    Cut,
    /// They stay where they were too (Alt).
    Copy,
}

/// The selected pixels of one image, ready to be moved by any offset.
#[derive(Debug)]
pub struct PixelMove {
    /// The pixels the move starts from (with an alpha channel added for a layer's).
    base: Arc<RasterImage>,
    to_document: Affine,
    to_image: Affine,
    selection: Arc<RasterImage>,
    selection_codec: Codec,
    /// `to_document` when it only moves by whole pixels, and the selection is stored as
    /// selections are: rows of the selection are then read directly.
    shift: Option<(i64, i64)>,
    /// The selection's bounds in the image's pixels, `[x0, y0, x1, y1)` within it; `None` when
    /// it selects nothing of the image.
    area: Option<[i64; 4]>,
    mode: MoveMode,
    /// The image is a coverage (a layer mask): the hole hides instead of being transparent.
    coverage: bool,
    codec: Codec,
    blender: Blender,
    /// Image linear RGB → working space, and back (identity for gray images).
    to_working: Mat3,
    from_working: Mat3,
}

impl PixelMove {
    /// The pixels of `base`, an image placed in the document by `to_document` (its whole
    /// transform, groups included), that `selection` selects; `coverage`: `base` is a layer
    /// mask (gray without alpha), else a layer's pixels, given an alpha channel first
    /// (lossless) for the hole. Partial coverage blends in `blend_space` (ADR 0012).
    pub fn new(
        base: Arc<RasterImage>,
        to_document: Affine,
        selection: &Selection,
        blend_space: BlendSpace,
        mode: MoveMode,
        coverage: bool,
    ) -> Result<Self, PaintError> {
        let to_image = to_document.inverse().ok_or(PaintError::InvalidTransform)?;
        let base = if coverage {
            if base.format().layout != ChannelLayout::Gray {
                return Err(PaintError::NotACoverage);
            }
            base
        } else {
            match base.with_alpha() {
                Some(with_alpha) => Arc::new(with_alpha?),
                None => base,
            }
        };
        let size = base.size();
        let area = crate::selection::bounds(selection.image()).and_then(|b| {
            let [x0, y0, x1, y1] = to_image.map_rect([
                f64::from(b.x),
                f64::from(b.y),
                b.right() as f64,
                b.bottom() as f64,
            ]);
            let clamp = |v: f64, end: u32| v.clamp(0.0, f64::from(end)) as i64;
            let area = [
                clamp(x0.floor(), size.width),
                clamp(y0.floor(), size.height),
                clamp(x1.ceil(), size.width),
                clamp(y1.ceil(), size.height),
            ];
            (area[0] < area[2] && area[1] < area[3]).then_some(area)
        });
        let (to_working, from_working) = if base.format().layout.is_gray() {
            (IDENTITY, IDENTITY)
        } else {
            let space = base.format().color_space;
            (
                space.matrix_to(&WORKING_SPACE),
                WORKING_SPACE.matrix_to(&space),
            )
        };
        let selection = Arc::clone(selection.image());
        let whole = |v: f64| v.is_finite() && v.fract() == 0.0 && v.abs() < 1e15;
        let [a, b, c, d, e, f] = to_document.to_array();
        let shift = ((a, b, c, d) == (1.0, 0.0, 0.0, 1.0)
            && whole(e)
            && whole(f)
            && selection.stored_format() == SELECTION_FORMAT)
            .then_some((e as i64, f as i64));
        Ok(Self {
            shift,
            codec: Codec::new(base.stored_format()),
            selection_codec: Codec::new(selection.stored_format()),
            selection,
            base,
            to_document,
            to_image,
            area,
            mode,
            coverage,
            blender: Blender::new(blend_space),
            to_working,
            from_working,
        })
    }

    /// The pixels the move starts from.
    pub fn base(&self) -> &Arc<RasterImage> {
        &self.base
    }

    /// The whole image pixels the pixels move by for a move of (`dx`, `dy`) document pixels:
    /// the same on a layer that is only moved, scaled by the layer's transform otherwise.
    pub fn image_offset(&self, dx: i64, dy: i64) -> (i64, i64) {
        let (ax, ay) = self.to_image.apply(dx as f64, dy as f64);
        let (bx, by) = self.to_image.apply(0.0, 0.0);
        ((ax - bx).round() as i64, (ay - by).round() as i64)
    }

    /// The image with the selected pixels moved by (`dx`, `dy`) document pixels (see
    /// [`Self::image_offset`]); what lands outside the image is dropped. Every tile the move
    /// does not reach is shared with the base; the others are computed on every core.
    pub fn image(&self, dx: i64, dy: i64) -> Result<Arc<RasterImage>, PaintError> {
        let (ox, oy) = self.image_offset(dx, dy);
        let Some([x0, y0, x1, y1]) = self.area else {
            return Ok(Arc::clone(&self.base));
        };
        if (ox, oy) == (0, 0) {
            return Ok(Arc::clone(&self.base));
        }
        let size = self.base.size();
        let (width, height) = (i64::from(size.width), i64::from(size.height));
        // Where the pixels land, within the image.
        let to = [
            (x0 + ox).clamp(0, width),
            (y0 + oy).clamp(0, height),
            (x1 + ox).clamp(0, width),
            (y1 + oy).clamp(0, height),
        ];
        let mut reached = BTreeSet::new();
        let t = TILE_SIZE as i64;
        let mut reach = |[x0, y0, x1, y1]: [i64; 4]| {
            if x0 >= x1 || y0 >= y1 {
                return;
            }
            for row in y0 / t..=(y1 - 1) / t {
                for col in x0 / t..=(x1 - 1) / t {
                    reached.insert(TileCoord {
                        col: col as u32,
                        row: row as u32,
                    });
                }
            }
        };
        reach(to);
        if self.mode == MoveMode::Cut {
            reach([x0, y0, x1, y1]);
        }
        let level = &self.base.levels()[0];
        let mut work: Vec<(TileCoord, Vec<u8>)> = reached
            .into_iter()
            .filter_map(|coord| Some((coord, level.tile(coord)?.to_vec())))
            .collect();
        parallel_for_each(&mut work, |(coord, tile)| {
            self.move_tile(*coord, tile, (ox, oy), to);
        });
        let replaced = work
            .into_iter()
            .map(|(coord, tile)| (coord, Arc::from(tile)))
            .collect();
        Ok(Arc::new(self.base.with_tiles(replaced)?))
    }

    /// Recompute tile `coord` (`tile` holding the base's) for a move of `offset` image pixels,
    /// the pixels landing within `to`; then pad it.
    fn move_tile(&self, coord: TileCoord, tile: &mut [u8], offset: (i64, i64), to: [i64; 4]) {
        let [x0, y0, x1, y1] = self.area.unwrap_or_default();
        let size = self.base.size();
        let level = &self.base.levels()[0];
        let bpp = self.codec.bytes_per_pixel;
        let width = (size.width - coord.col * TILE_SIZE).min(TILE_SIZE) as usize;
        let height = (size.height - coord.row * TILE_SIZE).min(TILE_SIZE) as usize;
        let left = i64::from(coord.col * TILE_SIZE);
        let cut = self.mode == MoveMode::Cut;
        // How much of each pixel of a row leaves, and how much of the moved one lands there.
        let mut leaves = vec![0.0f32; width];
        let mut lands = vec![0.0f32; width];
        // The columns of the tile within `[from, to)`, as indices.
        let columns = |from: i64, to: i64| {
            let first = (from - left).clamp(0, width as i64) as usize;
            let last = (to - left).clamp(0, width as i64) as usize;
            first..last.max(first)
        };
        let (leaving, landing) = (columns(x0, x1), columns(to[0], to[2]));
        for ty in 0..height {
            let y = i64::from(coord.row * TILE_SIZE) + ty as i64;
            leaves.fill(0.0);
            lands.fill(0.0);
            if cut && (y0..y1).contains(&y) && !leaving.is_empty() {
                let span = leaving.clone();
                self.coverage_row(left + span.start as i64, y, &mut leaves[span]);
            }
            if (to[1]..to[3]).contains(&y) && !landing.is_empty() {
                let span = landing.clone();
                let from = left + span.start as i64 - offset.0;
                self.coverage_row(from, y - offset.1, &mut lands[span]);
            }
            let sy = y - offset.1;
            for tx in leaving.start.min(landing.start)..leaving.end.max(landing.end) {
                let (leaves, lands) = (leaves[tx], lands[tx]);
                if leaves <= 0.0 && lands <= 0.0 {
                    continue;
                }
                // Landing implies a source pixel within the image.
                let moved: &[u8] = if lands > 0.0 {
                    let (sx, sy) = ((left + tx as i64 - offset.0) as usize, sy as usize);
                    let source = TileCoord {
                        col: (sx / T) as u32,
                        row: (sy / T) as u32,
                    };
                    let at = ((sy % T) * T + sx % T) * bpp;
                    match level.tile(source) {
                        Some(tile) => &tile[at..at + bpp],
                        None => continue,
                    }
                } else {
                    &[]
                };
                let px = &mut tile[(ty * T + tx) * bpp..][..bpp];
                if self.coverage {
                    self.land_coverage(px, moved, leaves, lands);
                } else {
                    self.land(px, moved, leaves, lands);
                }
            }
        }
        pad_tile(tile, width, height, bpp);
    }

    /// The selection's coverage of image pixels `x..x + out.len()` of row `y`. A layer only
    /// moved by whole pixels reads the selection's tiles directly; any other transform maps
    /// each pixel's center into the document.
    fn coverage_row(&self, x: i64, y: i64, out: &mut [f32]) {
        let size = self.selection.size();
        let (width, height) = (i64::from(size.width), i64::from(size.height));
        let Some((sx, sy)) = self.shift else {
            let reader = MaskReader {
                image: &self.selection,
                codec: &self.selection_codec,
            };
            for (i, v) in out.iter_mut().enumerate() {
                let (dx, dy) = self
                    .to_document
                    .apply((x + i as i64) as f64 + 0.5, y as f64 + 0.5);
                *v = reader.at(dx.floor(), dy.floor());
            }
            return;
        };
        out.fill(0.0);
        let dy = y + sy;
        if !(0..height).contains(&dy) {
            return;
        }
        let level = &self.selection.levels()[0];
        let mut i = 0;
        while i < out.len() {
            let dx = x + i as i64 + sx;
            if dx < 0 {
                i += (-dx) as usize;
                continue;
            }
            if dx >= width {
                break;
            }
            let (dx, row) = (dx as usize, dy as usize);
            let run = (T - dx % T).min(out.len() - i);
            let coord = TileCoord {
                col: (dx / T) as u32,
                row: (row / T) as u32,
            };
            if let Some(tile) = level.tile(coord) {
                let at = ((row % T) * T + dx % T) * 2;
                let (samples, _) = tile[at..at + run * 2].as_chunks::<2>();
                for (v, sample) in out[i..i + run].iter_mut().zip(samples) {
                    *v = f32::from(u16::from_ne_bytes(*sample)) / f32::from(u16::MAX);
                }
            }
            i += run;
        }
    }

    /// Pixel `px` of a layer losing `leaves` of itself, then `lands` of pixel `moved` composited
    /// over it.
    fn land(&self, px: &mut [u8], moved: &[u8], leaves: f32, lands: f32) {
        let codec = &self.codec;
        if lands >= 1.0 && (leaves >= 1.0 || codec.alpha(moved) >= 1.0) {
            // Exact: the moved pixel replaces it.
            px.copy_from_slice(moved);
            return;
        }
        if lands <= 0.0 {
            if leaves >= 1.0 {
                codec.write([0.0; 3], 0.0, px);
                return;
            }
            if codec.scale_alpha(px, 1.0 - leaves) {
                // Only the alpha changes, as the Eraser does (ADR 0027).
                return;
            }
        }
        let (below, alpha) = codec.read_mapped(px, &mut |v| v);
        let keep = f64::from(1.0 - leaves);
        let below = mat_vec(&self.to_working, below.map(f64::from));
        let mut dst = [
            below[0] * keep,
            below[1] * keep,
            below[2] * keep,
            f64::from(alpha) * keep,
        ];
        if lands > 0.0 {
            let (color, alpha) = codec.read_mapped(moved, &mut |v| v);
            let amount = f64::from(lands);
            let color = mat_vec(&self.to_working, color.map(f64::from));
            let src = [
                color[0] * amount,
                color[1] * amount,
                color[2] * amount,
                f64::from(alpha) * amount,
            ];
            self.blender.blend(BlendMode::Normal, &src, &mut dst);
        }
        let rgb = mat_vec(&self.from_working, [dst[0], dst[1], dst[2]]);
        codec.write(rgb.map(|v| v as f32), dst[3] as f32, px);
    }

    /// Pixel `px` of a mask hiding `leaves` of itself, then moving towards pixel `moved` by
    /// `lands`.
    fn land_coverage(&self, px: &mut [u8], moved: &[u8], leaves: f32, lands: f32) {
        let codec = &self.codec;
        if lands >= 1.0 {
            px.copy_from_slice(moved);
            return;
        }
        let here = codec.read_mapped(px, &mut |v| v).0[0] * (1.0 - leaves);
        let value = if lands > 0.0 {
            let there = codec.read_mapped(moved, &mut |v| v).0[0];
            here + (there - here) * lands
        } else {
            here
        };
        codec.write([value; 3], 1.0, px);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::PixelFormat;
    use crate::geom::Size;
    use crate::selection::{Combine, EdgeOptions, Shape, select_shape};

    /// A `size` image whose pixel (x, y) is `[x, y, 7, 255]`.
    fn gradient(size: Size) -> Arc<RasterImage> {
        let mut pixels = Vec::new();
        for y in 0..size.height {
            for x in 0..size.width {
                pixels.extend_from_slice(&[x as u8, y as u8, 7, 255]);
            }
        }
        Arc::new(RasterImage::from_pixels(size, PixelFormat::RGBA8_SRGB, &pixels).unwrap())
    }

    fn rectangle(canvas: Size, left: f64, top: f64, right: f64, bottom: f64) -> Selection {
        let shape = Shape::Rectangle {
            left,
            top,
            right,
            bottom,
        };
        let image = select_shape(
            canvas,
            None,
            &shape,
            EdgeOptions::default(),
            Combine::Replace,
        )
        .unwrap()
        .unwrap();
        Selection::new(Arc::new(image)).unwrap()
    }

    /// Stored bytes of pixel (`x`, `y`) of level 0.
    fn pixel(image: &RasterImage, x: u32, y: u32) -> Vec<u8> {
        let bpp = image.stored_format().bytes_per_pixel() as usize;
        let tile = image.levels()[0]
            .tile(TileCoord {
                col: x / TILE_SIZE,
                row: y / TILE_SIZE,
            })
            .unwrap();
        let at = ((y % TILE_SIZE) * TILE_SIZE + x % TILE_SIZE) as usize * bpp;
        tile[at..at + bpp].to_vec()
    }

    fn pixel_move(base: Arc<RasterImage>, to_document: Affine, selection: &Selection) -> PixelMove {
        PixelMove::new(
            base,
            to_document,
            selection,
            BlendSpace::Perceptual,
            MoveMode::Cut,
            false,
        )
        .unwrap()
    }

    #[test]
    fn cut_pixels_land_shifted_and_leave_a_transparent_hole() {
        let canvas = Size::new(600, 300);
        let selection = rectangle(canvas, 10.0, 10.0, 20.0, 15.0);
        let base = gradient(canvas);
        let moved = pixel_move(Arc::clone(&base), Affine::IDENTITY, &selection)
            .image(300, 100)
            .unwrap();
        // Moved exactly.
        assert_eq!(pixel(&moved, 310, 110), [10, 10, 7, 255]);
        assert_eq!(pixel(&moved, 319, 114), [19, 14, 7, 255]);
        // The hole, and what was outside the selection.
        assert_eq!(pixel(&moved, 10, 10), [0, 0, 0, 0]);
        assert_eq!(pixel(&moved, 19, 14), [0, 0, 0, 0]);
        assert_eq!(pixel(&moved, 20, 14), [20, 14, 7, 255]);
        assert_eq!(pixel(&moved, 320, 110), [64, 110, 7, 255]);
        // Tiles the move does not reach are the base's.
        let tile = |image: &RasterImage, col, row| {
            Arc::clone(image.levels()[0].tile(TileCoord { col, row }).unwrap())
        };
        assert!(Arc::ptr_eq(&tile(&base, 2, 1), &tile(&moved, 2, 1)));
        assert!(!Arc::ptr_eq(&tile(&base, 0, 0), &tile(&moved, 0, 0)));
    }

    #[test]
    fn a_copy_keeps_the_pixels_where_they_were() {
        let canvas = Size::new(64, 64);
        let selection = rectangle(canvas, 4.0, 4.0, 8.0, 8.0);
        let moving = PixelMove::new(
            gradient(canvas),
            Affine::IDENTITY,
            &selection,
            BlendSpace::Perceptual,
            MoveMode::Copy,
            false,
        )
        .unwrap();
        let moved = moving.image(2, 0).unwrap();
        assert_eq!(pixel(&moved, 4, 4), [4, 4, 7, 255]);
        assert_eq!(pixel(&moved, 6, 4), [4, 4, 7, 255]);
        assert_eq!(pixel(&moved, 9, 7), [7, 7, 7, 255]);
        assert_eq!(pixel(&moved, 10, 7), [10, 7, 7, 255]);
    }

    #[test]
    fn overlapping_moves_take_the_pixels_from_where_they_started() {
        let canvas = Size::new(64, 64);
        let selection = rectangle(canvas, 4.0, 4.0, 12.0, 8.0);
        let moved = pixel_move(gradient(canvas), Affine::IDENTITY, &selection)
            .image(3, 0)
            .unwrap();
        // Within the overlap, the pixel from 3 to the left; the start of the hole is empty.
        assert_eq!(pixel(&moved, 8, 5), [5, 5, 7, 255]);
        assert_eq!(pixel(&moved, 14, 5), [11, 5, 7, 255]);
        assert_eq!(pixel(&moved, 6, 5), [0, 0, 0, 0]);
        assert_eq!(pixel(&moved, 15, 5), [15, 5, 7, 255]);
    }

    #[test]
    fn a_moved_layer_is_read_where_it_lies_in_the_document() {
        let canvas = Size::new(64, 64);
        // The layer's pixel (x, y) lies at (x + 5, y + 3) in the document.
        let selection = rectangle(canvas, 10.0, 10.0, 12.0, 11.0);
        let moved = pixel_move(gradient(canvas), Affine::translation(5.0, 3.0), &selection)
            .image(1, 2)
            .unwrap();
        assert_eq!(pixel(&moved, 5, 7), [0, 0, 0, 0]);
        assert_eq!(pixel(&moved, 6, 9), [5, 7, 7, 255]);
        assert_eq!(pixel(&moved, 7, 9), [6, 7, 7, 255]);
    }

    #[test]
    fn a_scaled_layer_moves_by_its_own_pixels() {
        let canvas = Size::new(64, 64);
        let selection = rectangle(canvas, 0.0, 0.0, 8.0, 8.0);
        let moving = pixel_move(gradient(canvas), Affine::scale(2.0, 2.0), &selection);
        assert_eq!(moving.image_offset(10, -4), (5, -2));
        let moved = moving.image(10, 0).unwrap();
        // Document pixels 0..8 are image pixels 0..4.
        assert_eq!(pixel(&moved, 5, 0), [0, 0, 7, 255]);
        assert_eq!(pixel(&moved, 8, 3), [3, 3, 7, 255]);
        assert_eq!(pixel(&moved, 0, 0), [0, 0, 0, 0]);
        assert_eq!(pixel(&moved, 4, 0), [4, 0, 7, 255]);
    }

    #[test]
    fn soft_edges_move_in_part() {
        let canvas = Size::new(64, 64);
        // Half of column 8 is selected.
        let selection = rectangle(canvas, 4.0, 4.0, 8.5, 8.0);
        let base = Arc::new(
            RasterImage::from_pixels(
                canvas,
                PixelFormat::RGBA8_SRGB,
                &[200, 100, 50, 255].repeat(64 * 64),
            )
            .unwrap(),
        );
        let moved = pixel_move(base, Affine::IDENTITY, &selection)
            .image(0, 10)
            .unwrap();
        // Half left (32767 / 65535 of it selected): alpha halved, colors untouched.
        assert_eq!(pixel(&moved, 8, 5), [200, 100, 50, 127]);
        // Half landed over an opaque pixel of the same color.
        assert_eq!(pixel(&moved, 8, 15), [200, 100, 50, 255]);
        assert_eq!(pixel(&moved, 7, 15), [200, 100, 50, 255]);
    }

    #[test]
    fn a_mask_hides_its_hole() {
        let canvas = Size::new(32, 32);
        let gray = PixelFormat {
            layout: ChannelLayout::Gray,
            ..PixelFormat::RGBA8_SRGB
        };
        let mut values = vec![255u8; 32 * 32];
        values[2 * 32 + 2] = 100;
        let base = Arc::new(RasterImage::from_pixels(canvas, gray, &values).unwrap());
        let selection = rectangle(canvas, 2.0, 2.0, 4.0, 4.0);
        let moving = PixelMove::new(
            base,
            Affine::IDENTITY,
            &selection,
            BlendSpace::Perceptual,
            MoveMode::Cut,
            true,
        )
        .unwrap();
        let moved = moving.image(10, 0).unwrap();
        assert_eq!(pixel(&moved, 12, 2), [100]);
        assert_eq!(pixel(&moved, 13, 3), [255]);
        assert_eq!(pixel(&moved, 2, 2), [0]);
        assert_eq!(pixel(&moved, 4, 2), [255]);
    }

    #[test]
    fn nothing_moves_without_an_offset_or_outside_the_selection() {
        let canvas = Size::new(32, 32);
        let base = gradient(canvas);
        let selection = rectangle(canvas, 2.0, 2.0, 4.0, 4.0);
        let moving = pixel_move(Arc::clone(&base), Affine::IDENTITY, &selection);
        assert!(Arc::ptr_eq(&moving.image(0, 0).unwrap(), moving.base()));
        // A layer placed away from the selection.
        let away = pixel_move(base, Affine::translation(100.0, 0.0), &selection);
        assert!(Arc::ptr_eq(&away.image(5, 5).unwrap(), away.base()));
    }

    #[test]
    fn pixels_moved_beyond_the_image_are_dropped() {
        let canvas = Size::new(32, 32);
        let selection = rectangle(canvas, 28.0, 0.0, 32.0, 2.0);
        let moved = pixel_move(gradient(canvas), Affine::IDENTITY, &selection)
            .image(2, 0)
            .unwrap();
        assert_eq!(pixel(&moved, 30, 0), [28, 0, 7, 255]);
        assert_eq!(pixel(&moved, 31, 1), [29, 1, 7, 255]);
        assert_eq!(pixel(&moved, 28, 0), [0, 0, 0, 0]);
    }

    /// Timing of a move of a large selection on a photo-sized layer (run with `--release
    /// --ignored --nocapture`).
    #[test]
    #[ignore = "benchmark"]
    fn bench_move_large_selection() {
        let canvas = Size::new(6000, 4000);
        let base = Arc::new(
            RasterImage::from_pixels(
                canvas,
                PixelFormat::RGBA8_SRGB,
                &[200, 100, 50, 255].repeat(6000 * 4000),
            )
            .unwrap(),
        );
        let shape = Shape::Ellipse {
            left: 1000.0,
            top: 500.0,
            right: 5000.0,
            bottom: 3500.0,
        };
        let image = select_shape(
            canvas,
            None,
            &shape,
            EdgeOptions::default(),
            Combine::Replace,
        )
        .unwrap()
        .unwrap();
        let selection = Selection::new(Arc::new(image)).unwrap();
        let moving = pixel_move(base, Affine::IDENTITY, &selection);
        let started = std::time::Instant::now();
        let frames = 20;
        for i in 0..frames {
            moving.image(7 * i64::from(i + 1), 3).unwrap();
        }
        let per_frame = started.elapsed().as_secs_f64() * 1000.0 / f64::from(frames);
        let started = std::time::Instant::now();
        for i in 0..frames {
            crate::selection::translated(canvas, selection.image(), 7 * i64::from(i + 1), 3)
                .unwrap();
        }
        let outline = started.elapsed().as_secs_f64() * 1000.0 / f64::from(frames);
        println!(
            "a 4000 × 3000 ellipse moved: {per_frame:.1} ms per frame, its selection {outline:.1} ms"
        );
    }
}
