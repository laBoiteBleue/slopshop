//! Tiling of an image into fixed-size square tiles.
//!
//! This is only geometry: which tiles exist and which ones a region touches. It is the common
//! language for regions of interest, caches and out-of-core storage, none of which exist yet.

use std::num::NonZeroU32;

use crate::geom::{Rect, Size};

/// Position of a tile in the grid (column, row), not in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileCoord {
    pub col: u32,
    pub row: u32,
}

/// A grid of `tile_size × tile_size` tiles covering an image. Edge tiles are clipped to the
/// image bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileGrid {
    image: Size,
    tile_size: NonZeroU32,
}

impl TileGrid {
    pub const fn new(image: Size, tile_size: NonZeroU32) -> Self {
        Self { image, tile_size }
    }

    pub const fn image_size(&self) -> Size {
        self.image
    }

    pub const fn tile_size(&self) -> u32 {
        self.tile_size.get()
    }

    pub const fn columns(&self) -> u32 {
        self.image.width.div_ceil(self.tile_size.get())
    }

    pub const fn rows(&self) -> u32 {
        self.image.height.div_ceil(self.tile_size.get())
    }

    pub const fn tile_count(&self) -> u64 {
        self.columns() as u64 * self.rows() as u64
    }

    /// Pixel rectangle of a tile, clipped to the image. `None` if the tile is outside the grid.
    pub fn tile_rect(&self, coord: TileCoord) -> Option<Rect> {
        if coord.col >= self.columns() || coord.row >= self.rows() {
            return None;
        }
        let size = self.tile_size.get();
        // col < columns, so col * size < width + size: compute in u64 to be safe.
        let x = u64::from(coord.col) * u64::from(size);
        let y = u64::from(coord.row) * u64::from(size);
        let full = Rect::new(x as u32, y as u32, size, size);
        full.intersection(self.image.bounds())
    }

    /// All tiles touched by `region` (clipped to the image), in row-major order.
    pub fn tiles_intersecting(&self, region: Rect) -> impl Iterator<Item = TileCoord> + use<> {
        let size = self.tile_size.get();
        let (cols, rows) = match region.intersection(self.image.bounds()) {
            Some(r) => (
                r.x / size..r.right().div_ceil(u64::from(size)) as u32,
                r.y / size..r.bottom().div_ceil(u64::from(size)) as u32,
            ),
            None => (0..0, 0..0),
        };
        rows.flat_map(move |row| cols.clone().map(move |col| TileCoord { col, row }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(w: u32, h: u32, tile: u32) -> TileGrid {
        TileGrid::new(Size::new(w, h), NonZeroU32::new(tile).unwrap())
    }

    #[test]
    fn counts_round_up() {
        let g = grid(1000, 513, 256);
        assert_eq!((g.columns(), g.rows(), g.tile_count()), (4, 3, 12));
        assert_eq!(grid(0, 100, 256).tile_count(), 0);
    }

    #[test]
    fn edge_tiles_are_clipped() {
        let g = grid(1000, 513, 256);
        let c = |col, row| TileCoord { col, row };
        assert_eq!(g.tile_rect(c(0, 0)), Some(Rect::new(0, 0, 256, 256)));
        assert_eq!(g.tile_rect(c(3, 2)), Some(Rect::new(768, 512, 232, 1)));
        assert_eq!(g.tile_rect(c(4, 0)), None);
        assert_eq!(g.tile_rect(c(0, 3)), None);
    }

    #[test]
    fn tiles_intersecting_region() {
        let g = grid(1000, 1000, 256);
        let tiles: Vec<_> = g
            .tiles_intersecting(Rect::new(250, 10, 10, 300))
            .map(|t| (t.col, t.row))
            .collect();
        assert_eq!(tiles, vec![(0, 0), (1, 0), (0, 1), (1, 1)]);
    }

    #[test]
    fn region_aligned_on_tile_edge_does_not_touch_next_tile() {
        let g = grid(1024, 1024, 256);
        let tiles: Vec<_> = g.tiles_intersecting(Rect::new(0, 0, 256, 256)).collect();
        assert_eq!(tiles, vec![TileCoord { col: 0, row: 0 }]);
    }

    #[test]
    fn region_outside_or_clipped() {
        let g = grid(500, 500, 256);
        assert_eq!(g.tiles_intersecting(Rect::new(600, 600, 10, 10)).count(), 0);
        assert_eq!(
            g.tiles_intersecting(Rect::new(0, 0, 10_000, 10_000))
                .count(),
            4
        );
    }

    #[test]
    fn gigapixel_image_does_not_overflow() {
        let g = grid(u32::MAX, u32::MAX, 512);
        let last = TileCoord {
            col: g.columns() - 1,
            row: g.rows() - 1,
        };
        let rect = g.tile_rect(last).unwrap();
        assert_eq!(rect.right(), u64::from(u32::MAX));
        assert_eq!(rect.bottom(), u64::from(u32::MAX));
    }
}
