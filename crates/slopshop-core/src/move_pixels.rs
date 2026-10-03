//! Moving selected pixels: the Move tool dragged from inside a selection, as in Photoshop. The
//! selected pixels of an image leave a hole (transparent; hidden in a mask) and land shifted,
//! over what is there; Alt copies them instead (no hole). Soft selections move their pixels in
//! part, as Photoshop does.
//!
//! The result is a new image sharing every tile the move does not reach, computed from the
//! image the move started from, so moving again from there never compounds. It becomes the
//! layer's painted image (ADR 0027): the layer's original stays intact, and Delete Paint brings
//! it back. A layer's image grows, by whole tiles, to keep the pixels that land beyond it (off
//! the canvas too, as Photoshop keeps them); a mask keeps its size, as it covers its layer.
//!
//! While a drag goes on, a layer's pixels float ([`PixelMove::lift`], [`show_floating`]): the
//! view shows the layer as an isolated group of its pixels with the hole and of the selected
//! pixels alone, moved by a transform. A frame then costs what moving a layer costs, whatever
//! the image; the moved image is computed once, when the drag ends. Both composite the same, up
//! to the rounding of the pixels' format at soft edges.

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::blend::{BlendMode, BlendSpace, Blender};
use crate::color::{ChannelLayout, IDENTITY, Mat3, WORKING_SPACE, mat_vec};
use crate::document::{Document, Layer, LayerContent, LayerId, LayerMask};
use crate::edit::{Edit, EditError};
use crate::geom::Size;
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

/// Most pixels an image grows to so as to keep moved pixels (as for strokes).
const MAX_GROWN_PIXELS: u64 = 1 << 31;

/// How an image grows: whole tiles (columns, rows) added before its pixels, and its new size.
type Growth = ((u32, u32), Size);

/// A moved image.
#[derive(Debug, Clone)]
pub struct Moved {
    pub image: Arc<RasterImage>,
    /// Whole tiles (columns, rows) the image grew by before the base's pixels to keep what
    /// landed beyond it, if it grew (its size tells how much it grew after them).
    pub grown: Option<(u32, u32)>,
}

/// Selected pixels taken out of an image ([`PixelMove::extract`]).
#[derive(Debug, Clone)]
pub struct Extracted {
    pub image: Arc<RasterImage>,
    /// Where the image lies in the document: the base's place, moved to the first tile taken.
    pub to_document: Affine,
}

/// A layer's pixels lifted for a move (see [`show_floating`]).
#[derive(Debug, Clone)]
pub struct Lifted {
    /// The pixels with the hole the selected ones leave (the base itself for a copy).
    pub hole: Arc<RasterImage>,
    /// The selected pixels alone ([`PixelMove::extract`]), and where their image starts among
    /// the layer's pixels (whole tiles); `None` when nothing of the layer is selected.
    pub pixels: Option<(Arc<RasterImage>, (i64, i64))>,
}

/// An image the move works on and where it lies.
#[derive(Debug)]
struct Placement {
    base: Arc<RasterImage>,
    to_document: Affine,
    /// `to_document` when it only moves by whole pixels, and the selection is stored as
    /// selections are: rows of the selection are then read directly.
    shift: Option<(i64, i64)>,
    /// The selection's bounds in the image's pixels, `[x0, y0, x1, y1)` within it; `None` when
    /// it selects nothing of the image.
    area: Option<[i64; 4]>,
}

/// The selected pixels of one image, ready to be moved by any offset.
#[derive(Debug)]
pub struct PixelMove {
    /// The pixels the move starts from (with an alpha channel added for a layer's).
    placement: Arc<Placement>,
    /// The base grown to keep the pixels of the last move that needed it: its growth (tiles
    /// before, size), kept while the next moves need the same.
    grown: Option<((u32, u32), Size, Arc<Placement>)>,
    to_image: Affine,
    selection: Arc<RasterImage>,
    selection_codec: Codec,
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
            codec: Codec::new(base.stored_format()),
            selection_codec: Codec::new(selection.stored_format()),
            selection,
            placement: Arc::new(Placement {
                base,
                to_document,
                shift,
                area,
            }),
            grown: None,
            to_image,
            mode,
            coverage,
            blender: Blender::new(blend_space),
            to_working,
            from_working,
        })
    }

    /// The pixels the move starts from.
    pub fn base(&self) -> &Arc<RasterImage> {
        &self.placement.base
    }

    /// The whole image pixels the pixels move by for a move of (`dx`, `dy`) document pixels:
    /// the same on a layer that is only moved, scaled by the layer's transform otherwise.
    pub fn image_offset(&self, dx: i64, dy: i64) -> (i64, i64) {
        let (ax, ay) = self.to_image.apply(dx as f64, dy as f64);
        let (bx, by) = self.to_image.apply(0.0, 0.0);
        ((ax - bx).round() as i64, (ay - by).round() as i64)
    }

    /// The image with the selected pixels moved by (`dx`, `dy`) document pixels (see
    /// [`Self::image_offset`]). A layer's image grows to keep what lands beyond it (see
    /// [`Moved::grown`]); a mask drops it. Every tile the move does not reach is shared with
    /// the base; the others are computed on every core.
    pub fn image(&mut self, dx: i64, dy: i64) -> Result<Moved, PaintError> {
        let (ox, oy) = self.image_offset(dx, dy);
        let unmoved = Moved {
            image: Arc::clone(&self.placement.base),
            grown: None,
        };
        let Some([x0, y0, x1, y1]) = self.placement.area else {
            return Ok(unmoved);
        };
        if (ox, oy) == (0, 0) {
            return Ok(unmoved);
        }
        let growth = if self.coverage {
            None
        } else {
            self.growth([x0 + ox, y0 + oy, x1 + ox, y1 + oy])
        };
        let (placement, growth) = match growth {
            None => (Arc::clone(&self.placement), None),
            Some((offset, size)) => self.grown_placement(offset, size)?,
        };
        let Some([x0, y0, x1, y1]) = placement.area else {
            return Ok(unmoved);
        };
        let size = placement.base.size();
        let (width, height) = (i64::from(size.width), i64::from(size.height));
        // Where the pixels land, within the image.
        let to = [
            (x0 + ox).clamp(0, width),
            (y0 + oy).clamp(0, height),
            (x1 + ox).clamp(0, width),
            (y1 + oy).clamp(0, height),
        ];
        Ok(Moved {
            image: self.compose(&placement, (ox, oy), to)?,
            grown: growth.map(|(offset, _)| offset),
        })
    }

    /// The pixels split for a drag (see [`show_floating`]): the base with its hole, and the
    /// selected pixels alone. `None` for a mask, which has no transparency to show them with.
    pub fn lift(&self) -> Result<Option<Lifted>, PaintError> {
        if self.coverage {
            return Ok(None);
        }
        let hole = match self.mode {
            // Nothing lands: only the hole.
            MoveMode::Cut => self.compose(&self.placement, (0, 0), [0; 4])?,
            MoveMode::Copy => Arc::clone(&self.placement.base),
        };
        let base_to_document = self.placement.to_document;
        let pixels = self.extract()?.map(|extracted| {
            // Where the extracted image starts in the base: whole tiles, so exact.
            let start = extracted
                .to_document
                .then(base_to_document.inverse().unwrap_or(Affine::IDENTITY))
                .apply(0.0, 0.0);
            (
                extracted.image,
                (start.0.round() as i64, start.1.round() as i64),
            )
        });
        Ok(Some(Lifted { hole, pixels }))
    }

    /// `placement`'s base with the selected pixels moved by `offset` image pixels, landing
    /// within `to` (`[x0, y0, x1, y1)`, empty: nothing lands), leaving a hole for a cut. Every
    /// tile the move does not reach is shared with the base.
    fn compose(
        &self,
        placement: &Placement,
        offset: (i64, i64),
        to: [i64; 4],
    ) -> Result<Arc<RasterImage>, PaintError> {
        let Some([x0, y0, x1, y1]) = placement.area else {
            return Ok(Arc::clone(&placement.base));
        };
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
        let level = &placement.base.levels()[0];
        let mut work: Vec<(TileCoord, Vec<u8>)> = reached
            .into_iter()
            .filter_map(|coord| Some((coord, level.tile(coord)?.to_vec())))
            .collect();
        parallel_for_each(&mut work, |(coord, tile)| {
            self.move_tile(placement, *coord, tile, offset, to);
        });
        let replaced = work
            .into_iter()
            .map(|(coord, tile)| (coord, Arc::from(tile)))
            .collect();
        Ok(Arc::new(placement.base.with_tiles(replaced)?))
    }

    /// The selected pixels alone (Edit > Copy with a selection): the base's tiles the selection
    /// reaches, each pixel's alpha scaled by the selection's coverage, in the base's format, and
    /// where they lie in the document. Tiles wholly selected are shared with the base, so no
    /// pixel is resampled or copied needlessly; the image starts at a tile of the base, so it
    /// may hold transparent pixels around the selection. `None` when nothing of the base is
    /// selected. Only for a layer's pixels (not `coverage`).
    pub fn extract(&self) -> Result<Option<Extracted>, PaintError> {
        if self.coverage {
            return Err(PaintError::NotACoverage);
        }
        let placement = &self.placement;
        let Some([x0, y0, x1, y1]) = placement.area else {
            return Ok(None);
        };
        let t = i64::from(TILE_SIZE);
        let size = placement.base.size();
        let (first_col, first_row) = (x0 / t, y0 / t);
        let (cols, rows) = ((x1 - 1) / t - first_col + 1, (y1 - 1) / t - first_row + 1);
        let level = &placement.base.levels()[0];
        // Every tile of the area exists: the area lies within the base (a missing one would
        // make `from_level0_tiles` fail on the count).
        let mut work: Vec<(TileCoord, Arc<[u8]>)> = (0..rows)
            .flat_map(|row| (0..cols).map(move |col| (col, row)))
            .filter_map(|(col, row)| {
                let coord = TileCoord {
                    col: (first_col + col) as u32,
                    row: (first_row + row) as u32,
                };
                Some((coord, Arc::clone(level.tile(coord)?)))
            })
            .collect();
        parallel_for_each(&mut work, |(coord, tile)| {
            *tile = self.extract_tile(*coord, tile, [x0, y0, x1, y1]);
        });
        let tiles = work.into_iter().map(|(_, tile)| tile).collect();
        let (left, top) = (first_col * t, first_row * t);
        let extracted = Size::new(
            ((first_col + cols) * t).min(i64::from(size.width)) as u32 - left as u32,
            ((first_row + rows) * t).min(i64::from(size.height)) as u32 - top as u32,
        );
        let image = RasterImage::from_level0_tiles(extracted, placement.base.format(), tiles)?;
        Ok(Some(Extracted {
            image: Arc::new(image),
            to_document: Affine::translation(left as f64, top as f64).then(placement.to_document),
        }))
    }

    /// Tile `coord` of the base (`tile`) with each pixel's alpha scaled by the selection's
    /// coverage (`area` its bounds): the base's tile itself when it is wholly selected.
    fn extract_tile(&self, coord: TileCoord, tile: &Arc<[u8]>, area: [i64; 4]) -> Arc<[u8]> {
        let placement = &self.placement;
        let size = placement.base.size();
        let bpp = self.codec.bytes_per_pixel;
        let width = (size.width - coord.col * TILE_SIZE).min(TILE_SIZE) as usize;
        let height = (size.height - coord.row * TILE_SIZE).min(TILE_SIZE) as usize;
        let (left, top) = (
            i64::from(coord.col * TILE_SIZE),
            i64::from(coord.row * TILE_SIZE),
        );
        let mut coverage = vec![0.0f32; width];
        let mut out: Option<Vec<u8>> = None;
        for ty in 0..height {
            let y = top + ty as i64;
            coverage.fill(0.0);
            if (area[1]..area[3]).contains(&y) {
                self.coverage_row(placement, left, y, &mut coverage);
            }
            for (tx, &c) in coverage.iter().enumerate() {
                if c >= 1.0 {
                    continue;
                }
                let pixels = out.get_or_insert_with(|| tile.to_vec());
                let px = &mut pixels[(ty * T + tx) * bpp..][..bpp];
                if c <= 0.0 {
                    self.codec.write([0.0; 3], 0.0, px);
                } else if !self.codec.scale_alpha(px, c) {
                    let (color, alpha) = self.codec.read_mapped(px, &mut |v| v);
                    self.codec.write(color.map(|v| v * c), alpha * c, px);
                }
            }
        }
        match out {
            Some(mut pixels) => {
                pad_tile(&mut pixels, width, height, bpp);
                Arc::from(pixels)
            }
            None => Arc::clone(tile),
        }
    }

    /// How the base must grow for pixels landing in `to` (`[x0, y0, x1, y1)`, image pixels)
    /// to be kept: whole tiles before it (columns, rows) and its new size. `None` when they
    /// fit, or when it would grow beyond [`MAX_GROWN_PIXELS`] (what lands beyond is dropped).
    fn growth(&self, [x0, y0, x1, y1]: [i64; 4]) -> Option<Growth> {
        let size = self.placement.base.size();
        let (width, height) = (i64::from(size.width), i64::from(size.height));
        if x0 >= 0 && y0 >= 0 && x1 <= width && y1 <= height {
            return None;
        }
        // By steps of a quarter of the image in whole tiles, on both sides (transparent margins,
        // off the canvas): growing rebuilds the image's pyramid, so a drag does it rarely.
        let t = TILE_SIZE as i64;
        let steps = |beyond: i64, extent: i64| {
            let step = (extent as u64).div_ceil(4 * t as u64).max(1) * t as u64;
            (beyond.max(0) as u64).div_ceil(step) as i64 * step as i64
        };
        let left = steps(-x0, width) / t;
        let top = steps(-y0, height) / t;
        let grown_width = left * t + width + steps(x1 - width, width);
        let grown_height = top * t + height + steps(y1 - height, height);
        let fits = u32::try_from(grown_width).is_ok()
            && u32::try_from(grown_height).is_ok()
            && (grown_width as u64) * (grown_height as u64) <= MAX_GROWN_PIXELS;
        fits.then(|| {
            (
                (left as u32, top as u32),
                Size::new(grown_width as u32, grown_height as u32),
            )
        })
    }

    /// The base grown by `offset` whole tiles before it to `size` (zeros around it: transparent),
    /// kept for the next moves; or the one kept, when it holds as much around the base. With
    /// the growth it has.
    fn grown_placement(
        &mut self,
        offset: (u32, u32),
        size: Size,
    ) -> Result<(Arc<Placement>, Option<Growth>), PaintError> {
        let t = TILE_SIZE;
        // What a growth adds after the base's pixels, in pixels.
        let after = |(o, s): Growth| {
            let base = self.placement.base.size();
            (
                u64::from(s.width) - u64::from(o.0 * t) - u64::from(base.width),
                u64::from(s.height) - u64::from(o.1 * t) - u64::from(base.height),
            )
        };
        if let Some((o, s, placement)) = &self.grown {
            let (kept, needed) = (after((*o, *s)), after((offset, size)));
            if o.0 >= offset.0 && o.1 >= offset.1 && kept.0 >= needed.0 && kept.1 >= needed.1 {
                return Ok((Arc::clone(placement), Some((*o, *s))));
            }
        }
        let base = &self.placement;
        let grown = match base.base.grown(offset, size) {
            Some(grown) => Arc::new(grown?),
            None => return Ok((Arc::clone(base), None)),
        };
        let t = TILE_SIZE as i64;
        let (left, top) = (i64::from(offset.0) * t, i64::from(offset.1) * t);
        let placement = Arc::new(Placement {
            base: grown,
            // A grown pixel lies where the base's pixel `left`, `top` before it does.
            to_document: Affine::translation(-left as f64, -top as f64).then(base.to_document),
            shift: base.shift.map(|(x, y)| (x - left, y - top)),
            area: base
                .area
                .map(|[x0, y0, x1, y1]| [x0 + left, y0 + top, x1 + left, y1 + top]),
        });
        self.grown = Some((offset, size, Arc::clone(&placement)));
        Ok((placement, Some((offset, size))))
    }

    /// Recompute tile `coord` of `placement`'s base (`tile` holding its pixels) for a move of
    /// `offset` image pixels, the pixels landing within `to`; then pad it.
    fn move_tile(
        &self,
        placement: &Placement,
        coord: TileCoord,
        tile: &mut [u8],
        offset: (i64, i64),
        to: [i64; 4],
    ) {
        let [x0, y0, x1, y1] = placement.area.unwrap_or_default();
        let size = placement.base.size();
        let level = &placement.base.levels()[0];
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
                self.coverage_row(placement, left + span.start as i64, y, &mut leaves[span]);
            }
            if (to[1]..to[3]).contains(&y) && !landing.is_empty() {
                let span = landing.clone();
                let from = left + span.start as i64 - offset.0;
                self.coverage_row(placement, from, y - offset.1, &mut lands[span]);
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
    fn coverage_row(&self, placement: &Placement, x: i64, y: i64, out: &mut [f32]) {
        let size = self.selection.size();
        let (width, height) = (i64::from(size.width), i64::from(size.height));
        let Some((sx, sy)) = placement.shift else {
            let reader = MaskReader {
                image: &self.selection,
                codec: &self.selection_codec,
            };
            for (i, v) in out.iter_mut().enumerate() {
                let (dx, dy) = placement
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

/// Show layer `id` of `doc` with `lifted` pixels floating, moved by `offset` image pixels (see
/// [`PixelMove::image_offset`]): the view of a drag, never a document change (`doc` is a
/// snapshot). The layer becomes an isolated group with its id, name, visibility, opacity,
/// blend mode and clipping, placed by `transform` with `mask` (the layer as the move found
/// it, grown to the canvas if it needed to), holding the pixels with their hole and, above,
/// the selected pixels moved. An isolated group composites as one layer, also as a clipping
/// base (ADR 0015, 0016): it shows what the moved image will.
pub fn show_floating(
    doc: &mut Document,
    id: LayerId,
    lifted: &Lifted,
    offset: (i64, i64),
    transform: Affine,
    mask: Option<LayerMask>,
) -> Result<(), EditError> {
    let layer = doc.layer(id).ok_or(EditError::UnknownLayer(id))?.clone();
    let (parent, index) = doc.locate(id).ok_or(EditError::UnknownLayer(id))?;
    let part = |id: LayerId, image: &Arc<RasterImage>, transform: Affine| Layer {
        id,
        name: String::new(),
        visible: true,
        opacity: 1.0,
        blend_mode: BlendMode::Normal,
        content: LayerContent::raster(Arc::clone(image)),
        mask: None,
        clipped: false,
        transform,
    };
    let mut children = vec![part(
        doc.allocate_layer_id(),
        &lifted.hole,
        Affine::IDENTITY,
    )];
    if let Some((pixels, (x, y))) = &lifted.pixels {
        let moved = Affine::translation((x + offset.0) as f64, (y + offset.1) as f64);
        children.push(part(doc.allocate_layer_id(), pixels, moved));
    }
    let group = Layer {
        content: LayerContent::Group {
            children,
            pass_through: false,
        },
        mask,
        transform,
        ..layer
    };
    Edit::Batch(vec![
        Edit::RemoveLayer { id },
        Edit::InsertLayer {
            parent,
            index,
            layer: group,
        },
    ])
    .apply(doc)
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blend::BlendMode;
    use crate::color::{LinearRgba, PixelFormat};
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
            .unwrap()
            .image;
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
        let mut moving = PixelMove::new(
            gradient(canvas),
            Affine::IDENTITY,
            &selection,
            BlendSpace::Perceptual,
            MoveMode::Copy,
            false,
        )
        .unwrap();
        let moved = moving.image(2, 0).unwrap().image;
        assert_eq!(pixel(&moved, 4, 4), [4, 4, 7, 255]);
        assert_eq!(pixel(&moved, 6, 4), [4, 4, 7, 255]);
        assert_eq!(pixel(&moved, 9, 7), [7, 7, 7, 255]);
        assert_eq!(pixel(&moved, 10, 7), [10, 7, 7, 255]);
    }

    #[test]
    fn extracted_pixels_keep_their_place_and_their_alpha() {
        let canvas = Size::new(600, 300);
        let selection = rectangle(canvas, 300.0, 10.0, 310.0, 15.5);
        // The layer's pixel (x, y) lies at (x + 5, y + 3) in the document.
        let moving = pixel_move(gradient(canvas), Affine::translation(5.0, 3.0), &selection);
        let extracted = moving.extract().unwrap().unwrap();
        // From the layer's tile column 1 (x 256..512) and row 0, placed where they were.
        assert_eq!(extracted.image.size(), Size::new(256, 256));
        assert_eq!(extracted.to_document.apply(0.0, 0.0), (261.0, 3.0));
        // Document (300, 10) is layer pixel (295, 7), extracted pixel (39, 7).
        assert_eq!(pixel(&extracted.image, 39, 7), [39, 7, 7, 255]);
        assert_eq!(pixel(&extracted.image, 48, 11), [48, 11, 7, 255]);
        // Half covered on the last row, transparent outside.
        assert_eq!(pixel(&extracted.image, 48, 12)[3], 128);
        assert_eq!(pixel(&extracted.image, 38, 7), [0, 0, 0, 0]);
        assert_eq!(pixel(&extracted.image, 49, 7), [0, 0, 0, 0]);
        assert_eq!(pixel(&extracted.image, 39, 13), [0, 0, 0, 0]);
    }

    #[test]
    fn premultiplied_pixels_extracted_in_part_keep_their_color() {
        let canvas = Size::new(8, 8);
        let format = PixelFormat {
            layout: ChannelLayout::Rgba,
            sample: crate::color::SampleType::F32,
            color_space: crate::color::ColorSpace::LINEAR_SRGB,
            alpha: crate::color::AlphaMode::Premultiplied,
        };
        let pixel_bytes: Vec<u8> = [0.4f32, 0.2, 0.1, 0.8]
            .iter()
            .flat_map(|v| v.to_ne_bytes())
            .collect();
        let base =
            Arc::new(RasterImage::from_pixels(canvas, format, &pixel_bytes.repeat(64)).unwrap());
        // Column 2 is half selected.
        let selection = rectangle(canvas, 2.5, 0.0, 6.0, 8.0);
        let extracted = pixel_move(base, Affine::IDENTITY, &selection)
            .extract()
            .unwrap()
            .unwrap();
        let half: Vec<f32> = pixel(&extracted.image, 2, 0)
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_ne_bytes(*b))
            .collect();
        for (value, expected) in half.iter().zip([0.2, 0.1, 0.05, 0.4]) {
            assert!((value - expected).abs() < 1e-3, "{half:?}");
        }
    }

    #[test]
    fn wholly_selected_tiles_are_shared_and_nothing_selected_is_none() {
        let canvas = Size::new(600, 300);
        let base = gradient(canvas);
        let all = rectangle(canvas, 0.0, 0.0, 600.0, 300.0);
        let extracted = pixel_move(Arc::clone(&base), Affine::IDENTITY, &all)
            .extract()
            .unwrap()
            .unwrap();
        assert_eq!(extracted.image.size(), canvas);
        let tile = |image: &RasterImage| {
            Arc::clone(
                image.levels()[0]
                    .tile(TileCoord { col: 1, row: 0 })
                    .unwrap(),
            )
        };
        assert!(Arc::ptr_eq(&tile(&base), &tile(&extracted.image)));
        // A selection off the layer.
        let off = rectangle(canvas, 10.0, 10.0, 20.0, 20.0);
        let moving = pixel_move(base, Affine::translation(100.0, 100.0), &off);
        assert!(moving.extract().unwrap().is_none());
    }

    #[test]
    fn overlapping_moves_take_the_pixels_from_where_they_started() {
        let canvas = Size::new(64, 64);
        let selection = rectangle(canvas, 4.0, 4.0, 12.0, 8.0);
        let moved = pixel_move(gradient(canvas), Affine::IDENTITY, &selection)
            .image(3, 0)
            .unwrap()
            .image;
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
            .unwrap()
            .image;
        assert_eq!(pixel(&moved, 5, 7), [0, 0, 0, 0]);
        assert_eq!(pixel(&moved, 6, 9), [5, 7, 7, 255]);
        assert_eq!(pixel(&moved, 7, 9), [6, 7, 7, 255]);
    }

    #[test]
    fn a_scaled_layer_moves_by_its_own_pixels() {
        let canvas = Size::new(64, 64);
        let selection = rectangle(canvas, 0.0, 0.0, 8.0, 8.0);
        let mut moving = pixel_move(gradient(canvas), Affine::scale(2.0, 2.0), &selection);
        assert_eq!(moving.image_offset(10, -4), (5, -2));
        let moved = moving.image(10, 0).unwrap().image;
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
            .unwrap()
            .image;
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
        let mut moving = PixelMove::new(
            base,
            Affine::IDENTITY,
            &selection,
            BlendSpace::Perceptual,
            MoveMode::Cut,
            true,
        )
        .unwrap();
        let moved = moving.image(10, 0).unwrap().image;
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
        let mut moving = pixel_move(Arc::clone(&base), Affine::IDENTITY, &selection);
        let unmoved = moving.image(0, 0).unwrap();
        assert!(Arc::ptr_eq(&unmoved.image, moving.base()) && unmoved.grown.is_none());
        // A layer placed away from the selection.
        let mut away = pixel_move(base, Affine::translation(100.0, 0.0), &selection);
        let unmoved = away.image(5, 5).unwrap().image;
        assert!(Arc::ptr_eq(&unmoved, away.base()));
    }

    #[test]
    fn a_layer_grows_to_keep_pixels_moved_beyond_it() {
        let canvas = Size::new(32, 32);
        let selection = rectangle(canvas, 28.0, 0.0, 32.0, 2.0);
        let mut moving = pixel_move(gradient(canvas), Affine::IDENTITY, &selection);
        // Beyond the right edge: the image grows after its pixels.
        let moved = moving.image(2, 0).unwrap();
        assert_eq!(moved.grown, Some((0, 0)));
        assert_eq!(moved.image.size(), Size::new(32 + TILE_SIZE, 32));
        // Moving further within that growth keeps it.
        let further = moving.image(9, 0).unwrap();
        assert_eq!(
            (further.grown, further.image.size()),
            (moved.grown, moved.image.size())
        );
        assert_eq!(pixel(&moved.image, 32, 0), [30, 0, 7, 255]);
        assert_eq!(pixel(&moved.image, 33, 1), [31, 1, 7, 255]);
        assert_eq!(pixel(&moved.image, 28, 0), [0, 0, 0, 0]);
        assert_eq!(pixel(&moved.image, 32, 2), [0, 0, 0, 0]);
        // Beyond the left edge: whole tiles before them, the base's pixels shifted.
        let moved = moving.image(-40, 0).unwrap();
        assert_eq!(moved.grown, Some((1, 0)));
        let t = TILE_SIZE;
        assert_eq!(moved.image.size(), Size::new(t + 32, 32));
        assert_eq!(pixel(&moved.image, t - 12, 0), [28, 0, 7, 255]);
        assert_eq!(pixel(&moved.image, t + 28, 1), [0, 0, 0, 0]);
        assert_eq!(pixel(&moved.image, t + 27, 1), [27, 1, 7, 255]);
        assert_eq!(pixel(&moved.image, t - 20, 0), [0, 0, 0, 0]);
    }

    #[test]
    fn a_mask_keeps_its_size() {
        let canvas = Size::new(32, 32);
        let gray = PixelFormat {
            layout: ChannelLayout::Gray,
            ..PixelFormat::RGBA8_SRGB
        };
        let base = Arc::new(RasterImage::from_pixels(canvas, gray, &[255; 32 * 32]).unwrap());
        let selection = rectangle(canvas, 28.0, 0.0, 32.0, 2.0);
        let mut moving = PixelMove::new(
            base,
            Affine::IDENTITY,
            &selection,
            BlendSpace::Perceptual,
            MoveMode::Cut,
            true,
        )
        .unwrap();
        let moved = moving.image(2, 0).unwrap();
        assert_eq!((moved.grown, moved.image.size()), (None, canvas));
        assert_eq!(pixel(&moved.image, 31, 0), [255]);
        assert_eq!(pixel(&moved.image, 29, 0), [0]);
    }

    /// A document like a user's: a fill, layer `L` (colors and partial alpha, a layer only moved
    /// by whole pixels, 60 % opacity in Multiply, a mask hiding its right part) and a fill
    /// clipped to it; a soft selection.
    fn scene(placed: Affine) -> (Document, LayerId, Selection) {
        let canvas = Size::new(300, 200);
        let mut doc = Document::new(canvas);
        let fill = |doc: &mut Document, color: LinearRgba, clipped: bool| Layer {
            id: doc.allocate_layer_id(),
            name: String::new(),
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            content: LayerContent::Fill { color },
            mask: None,
            clipped,
            transform: Affine::IDENTITY,
        };
        let mut pixels = Vec::new();
        for y in 0..canvas.height {
            for x in 0..canvas.width {
                let alpha = if (x / 7 + y / 5) % 3 == 0 { 120 } else { 255 };
                pixels.extend_from_slice(&[(x % 256) as u8, (y * 2 % 256) as u8, 90, alpha]);
            }
        }
        let image = RasterImage::from_pixels(canvas, PixelFormat::RGBA8_SRGB, &pixels).unwrap();
        let shape = Shape::Rectangle {
            left: 0.0,
            top: 0.0,
            right: 230.0,
            bottom: 200.0,
        };
        let edges = EdgeOptions {
            anti_alias: true,
            feather: 6.0,
        };
        let hiding = select_shape(canvas, None, &shape, edges, Combine::Replace)
            .unwrap()
            .unwrap();
        let id = doc.allocate_layer_id();
        let layer = Layer {
            id,
            name: "L".into(),
            visible: true,
            opacity: 0.6,
            blend_mode: BlendMode::Multiply,
            content: LayerContent::raster(Arc::new(image)),
            mask: Some(LayerMask {
                image: Arc::new(hiding),
                enabled: true,
                replaces_alpha: false,
                original: None,
            }),
            clipped: false,
            transform: placed,
        };
        let below = fill(&mut doc, LinearRgba::new(0.2, 0.5, 0.8, 1.0), false);
        let clipped = fill(&mut doc, LinearRgba::new(0.9, 0.1, 0.1, 0.3), true);
        for (index, layer) in [below, layer, clipped].into_iter().enumerate() {
            Edit::InsertLayer {
                parent: None,
                index,
                layer,
            }
            .apply(&mut doc)
            .unwrap();
        }
        let shape = Shape::Ellipse {
            left: 40.0,
            top: 30.0,
            right: 160.0,
            bottom: 140.0,
        };
        let soft = select_shape(canvas, None, &shape, edges, Combine::Replace)
            .unwrap()
            .unwrap();
        (doc, id, Selection::new(Arc::new(soft)).unwrap())
    }

    fn composite(doc: &Document) -> Vec<f32> {
        let size = doc.size();
        let mut out = vec![0.0; size.pixel_count() as usize * 4];
        crate::composite::composite_region(doc, size.bounds(), &mut out).unwrap();
        out
    }

    /// The floating view of a move by (`dx`, `dy`) composites as its result does.
    fn floating_matches_the_result(placed: Affine, mode: MoveMode, dx: i64, dy: i64) {
        let (doc, id, selection) = scene(placed);
        let layer = doc.layer(id).unwrap().clone();
        let LayerContent::Raster { image, .. } = &layer.content else {
            unreachable!("a raster layer");
        };
        let mut moving = PixelMove::new(
            Arc::clone(image),
            layer.transform,
            &selection,
            doc.blend_space(),
            mode,
            false,
        )
        .unwrap();
        let lifted = moving.lift().unwrap().unwrap();
        let mut shown = doc.clone();
        let offset = moving.image_offset(dx, dy);
        show_floating(
            &mut shown,
            id,
            &lifted,
            offset,
            layer.transform,
            layer.mask.clone(),
        )
        .unwrap();
        let moved = moving.image(dx, dy).unwrap();
        assert!(moved.grown.is_none());
        let mut result = doc.clone();
        let stack = layer.content.stack().unwrap();
        Edit::bake_pixels(&doc, id, &stack, moving.base(), &moved.image)
            .unwrap()
            .apply(&mut result)
            .unwrap();
        let (a, b) = (composite(&shown), composite(&result));
        let worst = a
            .iter()
            .zip(&b)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        // 8-bit pixels round their soft edges: within a few levels of 255.
        assert!(worst < 0.012, "{worst}");
        // And the move shows: the views differ from the unmoved document.
        let unmoved = composite(&doc);
        assert!(a.iter().zip(&unmoved).any(|(a, b)| (a - b).abs() > 0.1));
    }

    #[test]
    fn floating_pixels_show_what_the_move_will_give() {
        floating_matches_the_result(Affine::IDENTITY, MoveMode::Cut, 37, 21);
        floating_matches_the_result(Affine::IDENTITY, MoveMode::Copy, -15, 40);
        floating_matches_the_result(Affine::translation(12.0, -5.0), MoveMode::Cut, 50, -9);
    }

    #[test]
    fn a_mask_does_not_float() {
        let canvas = Size::new(32, 32);
        let gray = PixelFormat {
            layout: ChannelLayout::Gray,
            ..PixelFormat::RGBA8_SRGB
        };
        let base = Arc::new(RasterImage::from_pixels(canvas, gray, &[255; 32 * 32]).unwrap());
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
        assert!(moving.lift().unwrap().is_none());
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
        let mut moving = pixel_move(base, Affine::IDENTITY, &selection);
        let started = std::time::Instant::now();
        moving.lift().unwrap();
        println!(
            "lifted once (a drag's start): {:.1} ms",
            started.elapsed().as_secs_f64() * 1000.0
        );
        let started = std::time::Instant::now();
        let frames = 20;
        for i in 0..frames {
            moving.image(7 * i64::from(i + 1), 3).unwrap();
        }
        let per_frame = started.elapsed().as_secs_f64() * 1000.0 / f64::from(frames);
        let started = std::time::Instant::now();
        for i in 0..frames {
            // Across the right edge, 37 px a frame.
            moving.image(1000 + 37 * i64::from(i), 3).unwrap();
        }
        let beyond = started.elapsed().as_secs_f64() * 1000.0 / f64::from(frames);
        println!("moved across the edge: {beyond:.1} ms per frame");
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
