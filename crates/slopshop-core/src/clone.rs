//! What the Clone Stamp paints from: a document as it shows (every visible layer, or one layer
//! alone), taken when the stroke starts and composited tile by tile the first time a dab
//! reaches it, in the working space (no 8-bit rounding). A stroke bakes what it took (ADR 0034,
//! point 6): the source never follows later changes.

use std::sync::{Arc, OnceLock};

use crate::document::Document;
use crate::geom::Rect;
use crate::raster::{TILE_SIZE, parallel_for_each};

/// A document's composited pixels, read by document pixel.
pub struct CloneSource {
    document: Document,
    /// Tiles of the canvas, row-major: premultiplied working-space RGBA, `TILE_SIZE²` pixels
    /// (the valid part; the rest transparent), composited once.
    tiles: Vec<OnceLock<Arc<[f32]>>>,
    columns: u32,
}

impl std::fmt::Debug for CloneSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ready = self.tiles.iter().filter(|t| t.get().is_some()).count();
        f.debug_struct("CloneSource")
            .field("size", &self.document.size())
            .field("ready_tiles", &ready)
            .finish()
    }
}

impl CloneSource {
    /// The pixels of `document` (shared: taking it copies no pixels).
    pub fn new(document: Document) -> Self {
        let size = document.size();
        let columns = size.width.div_ceil(TILE_SIZE);
        let rows = size.height.div_ceil(TILE_SIZE);
        let tiles = (0..columns * rows).map(|_| OnceLock::new()).collect();
        Self {
            document,
            tiles,
            columns,
        }
    }

    /// Composite the tiles `area` (document pixels, `[x0, y0, x1, y1)`) reaches that are not
    /// yet, on every core, so that [`Self::at`] finds them.
    pub fn prepare(&self, area: [f64; 4]) {
        let size = self.document.size();
        let t = f64::from(TILE_SIZE);
        let clamp = |v: f64, max: u32| (v / t).floor().clamp(0.0, f64::from(max)) as u32;
        let (columns, rows) = (self.columns, size.height.div_ceil(TILE_SIZE));
        if columns == 0 || rows == 0 || !area.iter().all(|v| v.is_finite()) {
            return;
        }
        let (c0, r0) = (clamp(area[0], columns - 1), clamp(area[1], rows - 1));
        let (c1, r1) = (clamp(area[2], columns - 1), clamp(area[3], rows - 1));
        if area[2] <= 0.0
            || area[3] <= 0.0
            || area[0] >= f64::from(size.width)
            || area[1] >= f64::from(size.height)
        {
            return;
        }
        let mut missing: Vec<(u32, u32)> = (r0..=r1)
            .flat_map(|row| (c0..=c1).map(move |col| (col, row)))
            .filter(|&(col, row)| self.tiles[(row * columns + col) as usize].get().is_none())
            .collect();
        parallel_for_each(&mut missing, |&mut (col, row)| {
            let cell = &self.tiles[(row * columns + col) as usize];
            cell.get_or_init(|| self.composite(col, row));
        });
    }

    fn composite(&self, col: u32, row: u32) -> Arc<[f32]> {
        let size = self.document.size();
        let (x, y) = (col * TILE_SIZE, row * TILE_SIZE);
        let (w, h) = (
            (size.width - x).min(TILE_SIZE),
            (size.height - y).min(TILE_SIZE),
        );
        let mut region = vec![0f32; (w * h * 4) as usize];
        // Spread over the tiles already: one thread each. Failed: transparent.
        if crate::composite::composite_region_serial(
            &self.document,
            Rect::new(x, y, w, h),
            &mut region,
        )
        .is_err()
        {
            region.fill(0.0);
        }
        let t = TILE_SIZE as usize;
        let mut tile = vec![0f32; t * t * 4];
        for (r, line) in region.chunks_exact(w as usize * 4).enumerate() {
            tile[r * t * 4..r * t * 4 + line.len()].copy_from_slice(line);
        }
        Arc::from(tile)
    }

    /// This source with the `width × height` pixels from document pixel (`left`, `top`)
    /// replaced by `colors` (premultiplied, row-major; those off the canvas dropped): the
    /// Healing Brush's blended colors. The tiles it reaches must be prepared.
    pub fn patched(
        &self,
        left: i64,
        top: i64,
        width: usize,
        height: usize,
        colors: &[[f32; 4]],
    ) -> CloneSource {
        let size = self.document.size();
        let t = TILE_SIZE as usize;
        let mut tiles: Vec<Option<Vec<f32>>> = self
            .tiles
            .iter()
            .map(|cell| cell.get().map(|tile| tile.to_vec()))
            .collect();
        for (row, line) in colors.chunks_exact(width.max(1)).enumerate().take(height) {
            let y = top + row as i64;
            if y < 0 || y >= i64::from(size.height) {
                continue;
            }
            for (col, color) in line.iter().enumerate() {
                let x = left + col as i64;
                if x < 0 || x >= i64::from(size.width) {
                    continue;
                }
                let (x, y) = (x as usize, y as usize);
                let index = (y / t) * self.columns as usize + x / t;
                if let Some(tile) = &mut tiles[index] {
                    let i = ((y % t) * t + x % t) * 4;
                    tile[i..i + 4].copy_from_slice(color);
                }
            }
        }
        CloneSource {
            document: self.document.clone(),
            tiles: tiles
                .into_iter()
                .map(|tile| {
                    let cell = OnceLock::new();
                    if let Some(tile) = tile {
                        let _ = cell.set(Arc::from(tile));
                    }
                    cell
                })
                .collect(),
            columns: self.columns,
        }
    }

    /// The premultiplied working-space color at document point (`x`, `y`) (the pixel it falls
    /// in): transparent off the canvas, or in a tile not prepared.
    pub fn at(&self, x: f64, y: f64) -> [f32; 4] {
        let size = self.document.size();
        if !(x >= 0.0 && y >= 0.0 && x < f64::from(size.width) && y < f64::from(size.height)) {
            return [0.0; 4];
        }
        let (px, py) = (x as u32, y as u32);
        let index = (py / TILE_SIZE * self.columns + px / TILE_SIZE) as usize;
        let Some(tile) = self.tiles[index].get() else {
            return [0.0; 4];
        };
        let i = (((py % TILE_SIZE) * TILE_SIZE + px % TILE_SIZE) * 4) as usize;
        [tile[i], tile[i + 1], tile[i + 2], tile[i + 3]]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::PixelFormat;
    use crate::document::{Layer, LayerContent};
    use crate::geom::Size;
    use crate::raster::RasterImage;

    /// A document of one 300 × 20 layer, red on its left half and transparent on its right.
    fn half_red() -> Document {
        let mut pixels = Vec::new();
        for _ in 0..20 {
            for x in 0..300 {
                pixels.extend_from_slice(if x < 150 {
                    &[255, 0, 0, 255]
                } else {
                    &[0, 0, 0, 0]
                });
            }
        }
        let image =
            RasterImage::from_pixels(Size::new(300, 20), PixelFormat::RGBA8_SRGB, &pixels).unwrap();
        let mut doc = Document::new(Size::new(300, 20));
        let layer = Layer {
            style: None,
            id: doc.allocate_layer_id(),
            name: "red".into(),
            visible: true,
            opacity: 1.0,
            blend_mode: crate::blend::BlendMode::Normal,
            mask: None,
            clipped: false,
            transform: crate::transform::Affine::IDENTITY,
            content: LayerContent::raster(Arc::new(image)),
        };
        crate::edit::Edit::InsertLayer {
            parent: None,
            index: 0,
            layer,
        }
        .apply(&mut doc)
        .unwrap();
        doc
    }

    #[test]
    fn pixels_are_read_once_their_tiles_are_prepared() {
        let source = CloneSource::new(half_red());
        // Not prepared yet: transparent.
        assert_eq!(source.at(10.0, 10.0), [0.0; 4]);
        source.prepare([0.0, 0.0, 20.0, 20.0]);
        // Red in the working space, opaque.
        let red = crate::color::LinearRgba::from_srgb_encoded_to_working(1.0, 0.0, 0.0, 1.0);
        let got = source.at(10.5, 10.5);
        for (v, want) in got.iter().zip([red.r, red.g, red.b, 1.0]) {
            assert!((v - want).abs() < 1e-3, "{got:?} != {red:?}");
        }
        // The tile beyond the first is still to come; then transparent where the layer is.
        assert_eq!(source.at(290.0, 10.0), [0.0; 4]);
        source.prepare([0.0, 0.0, 300.0, 20.0]);
        assert_eq!(source.at(290.0, 10.0)[3], 0.0);
        assert!(source.at(149.0, 10.0)[3] > 0.99);
        // Off the canvas, and nonsense areas.
        assert_eq!(source.at(-1.0, 5.0), [0.0; 4]);
        assert_eq!(source.at(5.0, 20.0), [0.0; 4]);
        source.prepare([f64::NAN, 0.0, 1.0, 1.0]);
        source.prepare([-50.0, -50.0, -1.0, -1.0]);
    }

    #[test]
    fn the_clone_stamp_paints_the_pixels_taken_offset_away() {
        use crate::blend::BlendSpace;
        use crate::paint::{Brush, Paint, PointerSample, Stroke};
        use crate::stack::LayerStack;
        use crate::transform::Affine;
        let doc = half_red();
        let layer = &doc.layers()[0];
        let shown = layer.content.pixels().unwrap();
        let stack = LayerStack::new(Arc::clone(&shown));
        let source = Arc::new(CloneSource::new(doc.clone()));
        let brush = Brush {
            diameter: 10.0,
            hardness: 1.0,
            ..Brush::default()
        };
        // Painting at x = 200 takes the pixels at x = 50: red.
        let mut s = Stroke::on_stack(
            &stack,
            Arc::clone(&shown),
            Affine::IDENTITY,
            None,
            BlendSpace::Perceptual,
            brush,
            Paint::Clone {
                offset: [-150.0, 0.0],
                gray: false,
            },
        )
        .unwrap()
        .cloning(Arc::clone(&source));
        s.add(&[PointerSample {
            x: 200.5,
            y: 10.5,
            pressure: 1.0,
        }]);
        let (stack, painted) = s.finish_stack().unwrap().unwrap();
        assert_eq!(stack.entries().len(), 1);
        let red = |x: u32| {
            let codec = crate::raster::Codec::new(painted.stored_format());
            let tile = painted.levels()[0]
                .tile(crate::tile::TileCoord {
                    col: x / TILE_SIZE,
                    row: 0,
                })
                .unwrap();
            let bpp = codec.bytes_per_pixel;
            let at = ((10 * TILE_SIZE + x % TILE_SIZE) as usize) * bpp;
            tile[at..at + bpp].to_vec()
        };
        assert_eq!(red(200), vec![255, 0, 0, 255]);
        // Away from the dab, the layer is as it was (transparent there).
        assert_eq!(red(260)[3], 0);
        // Taking from where the layer is transparent paints nothing.
        let mut empty = Stroke::on_stack(
            &LayerStack::new(Arc::clone(&shown)),
            Arc::clone(&shown),
            Affine::IDENTITY,
            None,
            BlendSpace::Perceptual,
            brush,
            Paint::Clone {
                offset: [80.0, 0.0],
                gray: false,
            },
        )
        .unwrap()
        .cloning(source);
        empty.add(&[PointerSample {
            x: 200.5,
            y: 10.5,
            pressure: 1.0,
        }]);
        let (_, untouched) = empty.finish_stack().unwrap().unwrap();
        let tile = untouched.levels()[0]
            .tile(crate::tile::TileCoord { col: 0, row: 0 })
            .unwrap();
        let bpp = untouched.stored_format().bytes_per_pixel() as usize;
        let at = (10 * TILE_SIZE as usize + 200) * bpp;
        assert_eq!(tile[at + bpp - 1], 0);
    }

    #[test]
    fn healing_takes_the_tone_around_where_it_paints() {
        use crate::blend::BlendSpace;
        use crate::paint::{Brush, Paint, PointerSample, Stroke};
        use crate::stack::LayerStack;
        use crate::transform::Affine;
        // Dark on the left, light on the right with a red speck at (220, 10).
        let mut pixels = Vec::new();
        for y in 0..20 {
            for x in 0..300 {
                let speck = (218..223).contains(&x) && (8..13).contains(&y);
                pixels.extend_from_slice(match (x < 150, speck) {
                    (_, true) => &[255, 0, 0, 255],
                    (true, false) => &[50, 50, 50, 255],
                    (false, false) => &[200, 200, 200, 255],
                });
            }
        }
        let image = Arc::new(
            RasterImage::from_pixels(Size::new(300, 20), PixelFormat::RGBA8_SRGB, &pixels).unwrap(),
        );
        let mut doc = Document::new(Size::new(300, 20));
        let layer = Layer {
            style: None,
            id: doc.allocate_layer_id(),
            name: "l".into(),
            visible: true,
            opacity: 1.0,
            blend_mode: crate::blend::BlendMode::Normal,
            mask: None,
            clipped: false,
            transform: Affine::IDENTITY,
            content: LayerContent::raster(Arc::clone(&image)),
        };
        crate::edit::Edit::InsertLayer {
            parent: None,
            index: 0,
            layer,
        }
        .apply(&mut doc)
        .unwrap();
        let source = Arc::new(CloneSource::new(doc));
        let brush = Brush {
            diameter: 12.0,
            hardness: 1.0,
            ..Brush::default()
        };
        let mut s = Stroke::on_stack(
            &LayerStack::new(Arc::clone(&image)),
            Arc::clone(&image),
            Affine::IDENTITY,
            None,
            BlendSpace::Perceptual,
            brush,
            // From the dark half.
            Paint::Clone {
                offset: [-150.0, 0.0],
                gray: false,
            },
        )
        .unwrap()
        .cloning(source);
        s.add(&[PointerSample {
            x: 220.5,
            y: 10.5,
            pressure: 1.0,
        }]);
        s.image().unwrap();
        s.heal().unwrap();
        let (_, healed) = s.finish_stack().unwrap().unwrap();
        let tile = healed.levels()[0]
            .tile(crate::tile::TileCoord { col: 0, row: 0 })
            .unwrap();
        let bpp = healed.stored_format().bytes_per_pixel() as usize;
        let at = (10 * TILE_SIZE as usize + 220) * bpp;
        let px = &tile[at..at + bpp];
        // The light gray around it, not the source's dark nor the speck's red.
        assert!(px[..3].iter().all(|&v| v.abs_diff(200) <= 6), "{px:?}");
    }
}
