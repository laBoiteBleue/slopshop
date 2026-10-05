//! What the Clone Stamp paints from: a document as it shows (every visible layer, or one layer
//! alone), taken when the stroke starts and composited tile by tile the first time a dab
//! reaches it, in the working space (no 8-bit rounding). A stroke bakes what it took (ADR 0034,
//! point 6): the source never follows later changes.

use std::sync::{Arc, OnceLock};

use crate::document::Document;
use crate::geom::Rect;
use crate::raster::{TILE_SIZE, parallel_for_each};

/// The Dodge and Burn tools: the colors taken (the layer as it was) lightened or darkened in a
/// range of tones, by `exposure` in `[0, 1]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tone {
    pub burn: bool,
    pub range: ToneRange,
    pub exposure: f32,
}

/// Which tones the Dodge and Burn tools change most (Photoshop's Range).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToneRange {
    Shadows,
    Midtones,
    Highlights,
}

impl Tone {
    /// `exposure` in `[0, 1]`.
    pub fn is_valid(&self) -> bool {
        (0.0..=1.0).contains(&self.exposure)
    }

    /// `color` (linear working-space RGB) lightened or darkened: per channel, on its encoded
    /// value (GIMP's dodge and burn curves): shadows lift or sink the dark end, midtones bend
    /// the middle (a power), highlights scale the bright end.
    pub fn apply(&self, color: [f32; 3]) -> [f32; 3] {
        let e = self.exposure;
        color.map(|c| {
            let v = crate::color::srgb_encode(c.clamp(0.0, 1.0));
            let third = e / 3.0;
            let out = match (self.burn, self.range) {
                (false, ToneRange::Highlights) => v * (1.0 + third),
                (false, ToneRange::Midtones) => v.powf(1.0 / (1.0 + e)),
                (false, ToneRange::Shadows) => third + v - third * v,
                (true, ToneRange::Highlights) => v * (1.0 - third),
                (true, ToneRange::Midtones) => v.powf(1.0 + e),
                (true, ToneRange::Shadows) => {
                    if third >= 1.0 {
                        0.0
                    } else {
                        (v - third) / (1.0 - third)
                    }
                }
            };
            crate::color::srgb_decode(out.clamp(0.0, 1.0))
        })
    }
}

/// The Blur and Sharpen tools: the source seen through a Gaussian blur of `sigma` pixels, or,
/// with `sharpen`, sharpened by that amount (an unsharp mask: the pixels plus the amount times
/// their difference with the blur).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceFilter {
    pub sigma: f32,
    pub sharpen: Option<f32>,
}

impl SourceFilter {
    /// A blur of 0.1 to 64 pixels, a sharpening of 0 to 10.
    pub fn is_valid(&self) -> bool {
        (0.1..=64.0).contains(&self.sigma) && self.sharpen.is_none_or(|a| (0.0..=10.0).contains(&a))
    }

    /// Pixels a tile needs around it.
    fn margin(&self) -> u32 {
        (self.sigma * 3.0).ceil() as u32
    }
}

/// A document's composited pixels, read by document pixel.
pub struct CloneSource {
    document: Document,
    /// Seen through a filter (the Blur and Sharpen tools).
    filter: Option<SourceFilter>,
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
            filter: None,
            tiles,
            columns,
        }
    }

    /// This source seen through `filter`, before any tile is prepared.
    pub fn filtered(mut self, filter: SourceFilter) -> Self {
        self.filter = Some(filter);
        self
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
        if let Some(filter) = self.filter {
            return self.composite_filtered(col, row, filter);
        }
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
            filter: self.filter,
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

    /// Tile (`col`, `row`) through `filter`: composited with the margin the blur reads (within
    /// the canvas: its edge repeats), blurred, sharpened if asked.
    fn composite_filtered(&self, col: u32, row: u32, filter: SourceFilter) -> Arc<[f32]> {
        let size = self.document.size();
        let m = filter.margin();
        let (x, y) = (col * TILE_SIZE, row * TILE_SIZE);
        let (x0, y0) = (x.saturating_sub(m), y.saturating_sub(m));
        let x1 = (x + TILE_SIZE + m).min(size.width);
        let y1 = (y + TILE_SIZE + m).min(size.height);
        let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
        let mut region = vec![0f32; w * h * 4];
        if crate::composite::composite_region_serial(
            &self.document,
            Rect::new(x0, y0, w as u32, h as u32),
            &mut region,
        )
        .is_err()
        {
            region.fill(0.0);
        }
        let blurred = gaussian(&region, w, h, filter.sigma);
        let t = TILE_SIZE as usize;
        let mut tile = vec![0f32; t * t * 4];
        let (ox, oy) = ((x - x0) as usize, (y - y0) as usize);
        let (vw, vh) = (
            ((size.width - x).min(TILE_SIZE)) as usize,
            ((size.height - y).min(TILE_SIZE)) as usize,
        );
        for r in 0..vh {
            for c in 0..vw {
                let i = ((oy + r) * w + ox + c) * 4;
                let o = (r * t + c) * 4;
                for k in 0..4 {
                    let v = match filter.sharpen {
                        Some(amount) => region[i + k] + amount * (region[i + k] - blurred[i + k]),
                        None => blurred[i + k],
                    };
                    tile[o + k] = v;
                }
                // Within what premultiplied colors can be.
                let a = tile[o + 3].clamp(0.0, 1.0);
                tile[o + 3] = a;
                for k in 0..3 {
                    tile[o + k] = tile[o + k].max(0.0);
                }
            }
        }
        Arc::from(tile)
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

/// `rgba` (`width × height` premultiplied pixels) blurred by a Gaussian of `sigma` pixels, in
/// two passes, the edge repeated.
fn gaussian(rgba: &[f32], width: usize, height: usize, sigma: f32) -> Vec<f32> {
    let radius = (sigma * 3.0).ceil() as isize;
    let weights: Vec<f32> = (-radius..=radius)
        .map(|d| (-(d * d) as f32 / (2.0 * sigma * sigma)).exp())
        .collect();
    let total: f32 = weights.iter().sum();
    let weights: Vec<f32> = weights.iter().map(|w| w / total).collect();
    let pass = |input: &[f32], horizontal: bool| {
        let mut out = vec![0f32; input.len()];
        for y in 0..height {
            for x in 0..width {
                let mut sum = [0f32; 4];
                for (k, w) in weights.iter().enumerate() {
                    let d = k as isize - radius;
                    let (sx, sy) = if horizontal {
                        ((x as isize + d).clamp(0, width as isize - 1) as usize, y)
                    } else {
                        (x, (y as isize + d).clamp(0, height as isize - 1) as usize)
                    };
                    let i = (sy * width + sx) * 4;
                    for c in 0..4 {
                        sum[c] += w * input[i + c];
                    }
                }
                out[(y * width + x) * 4..(y * width + x) * 4 + 4].copy_from_slice(&sum);
            }
        }
        out
    };
    if width == 0 || height == 0 {
        return rgba.to_vec();
    }
    pass(&pass(rgba, true), false)
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
                tone: None,
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
                tone: None,
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
                tone: None,
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

    #[test]
    fn dodge_lightens_and_burn_darkens_their_range_most() {
        let gray = |v: f32| [crate::color::srgb_decode(v); 3];
        let encoded = |c: [f32; 3]| crate::color::srgb_encode(c[0]);
        for range in [
            ToneRange::Shadows,
            ToneRange::Midtones,
            ToneRange::Highlights,
        ] {
            let dodge = Tone {
                burn: false,
                range,
                exposure: 0.5,
            };
            let burn = Tone {
                burn: true,
                ..dodge
            };
            for v in [0.2, 0.5, 0.8] {
                assert!(encoded(dodge.apply(gray(v))) > v, "{range:?} {v}");
                assert!(encoded(burn.apply(gray(v))) < v, "{range:?} {v}");
            }
            // No exposure: no change.
            let none = Tone {
                exposure: 0.0,
                ..dodge
            };
            assert!((encoded(none.apply(gray(0.5))) - 0.5).abs() < 1e-5);
        }
        // Midtones change the middle more than the ends; white stays white.
        let mid = Tone {
            burn: false,
            range: ToneRange::Midtones,
            exposure: 0.5,
        };
        let lift = |v: f32| encoded(mid.apply(gray(v))) - v;
        assert!(lift(0.5) > lift(0.95));
        assert!((encoded(mid.apply(gray(1.0))) - 1.0).abs() < 1e-5);
        assert!(
            !Tone {
                exposure: 1.5,
                ..mid
            }
            .is_valid()
        );
    }

    #[test]
    fn a_filtered_source_is_blurred_or_sharpened() {
        let blurred = CloneSource::new(half_red()).filtered(SourceFilter {
            sigma: 3.0,
            sharpen: None,
        });
        blurred.prepare([0.0, 0.0, 300.0, 20.0]);
        // Far from the edge: as it was; on it: half covered.
        assert!(blurred.at(50.5, 10.5)[3] > 0.99);
        assert!((blurred.at(150.0, 10.5)[3] - 0.5).abs() < 0.1);
        assert!(blurred.at(146.5, 10.5)[3] < 0.99);
        // Sharpened: beyond the edge on both sides (an unsharp mask's halo), clamped.
        let sharp = CloneSource::new(half_red()).filtered(SourceFilter {
            sigma: 2.0,
            sharpen: Some(1.0),
        });
        sharp.prepare([0.0, 0.0, 300.0, 20.0]);
        let inside = sharp.at(148.5, 10.5);
        assert_eq!(inside[3], 1.0, "alpha clamped");
        assert!(inside[0] > blurred.at(148.5, 10.5)[0]);
        assert!(
            !SourceFilter {
                sigma: 0.0,
                sharpen: None
            }
            .is_valid()
        );
    }
}
