//! The Histogram panel's counts (ADR 0036): how many pixels of the visible image have each
//! value, as displayed (whole 8-bit sRGB values, the eyedropper's), for red, green, blue and
//! luminosity (Photoshop's `0.30 R + 0.59 G + 0.11 B`). Within the selection when there is one
//! (each pixel counted by its coverage, as Photoshop does); transparent pixels are not counted.
//! Large images are sampled on a regular grid of rows and columns, up to [`MAX_SAMPLES`]
//! pixels: a histogram drawn in a panel needs no more.

use crate::composite::PixelCompositor;
use crate::document::Document;
use crate::paint::MaskReader;
use crate::raster::{Codec, parallel_for_each};
use crate::selection::WandSampler;

/// Pixels counted at most.
pub const MAX_SAMPLES: u64 = 1 << 18;

/// Counts per value (0–255), and how they were made.
#[derive(Debug, Clone, PartialEq)]
pub struct Histogram {
    pub red: [f64; 256],
    pub green: [f64; 256],
    pub blue: [f64; 256],
    pub luminosity: [f64; 256],
    /// Every `step`-th row and column was counted (1: every pixel).
    pub step: u32,
}

impl Histogram {
    fn empty(step: u32) -> Self {
        Self {
            red: [0.0; 256],
            green: [0.0; 256],
            blue: [0.0; 256],
            luminosity: [0.0; 256],
            step,
        }
    }

    fn add(&mut self, other: &Self) {
        for (mine, theirs) in [
            (&mut self.red, &other.red),
            (&mut self.green, &other.green),
            (&mut self.blue, &other.blue),
            (&mut self.luminosity, &other.luminosity),
        ] {
            for (a, b) in mine.iter_mut().zip(theirs) {
                *a += b;
            }
        }
    }
}

/// Photoshop's luminosity of 8-bit values.
fn luminosity(r: f32, g: f32, b: f32) -> usize {
    (0.30 * r + 0.59 * g + 0.11 * b).round().clamp(0.0, 255.0) as usize
}

/// The histogram of `document`'s visible image (within its selection when there is one).
pub fn histogram(document: &Document) -> Histogram {
    let canvas = document.size().bounds();
    let selection = document.selection();
    let region = match selection {
        Some(s) => crate::selection::bounds(s.image()).and_then(|b| b.intersection(canvas)),
        None => Some(canvas),
    };
    let Some(region) = region.filter(|r| !r.is_empty()) else {
        return Histogram::empty(1);
    };
    let pixels = region.size().pixel_count();
    let step = (pixels.div_ceil(MAX_SAMPLES) as f64).sqrt().ceil().max(1.0) as u32;
    // The CPU compositor, for the pixels counted only: rows one pixel high would cost a GPU
    // round trip each.
    let sampler = WandSampler::new(document, None);
    let compositor = PixelCompositor::new(document);
    let mask = selection.map(|s| (s.image().as_ref(), Codec::new(s.image().stored_format())));
    let rows: Vec<u32> = (region.y..region.y + region.height)
        .step_by(step as usize)
        .collect();
    let mut per_row: Vec<(u32, Histogram)> = rows
        .into_iter()
        .map(|y| (y, Histogram::empty(step)))
        .collect();
    parallel_for_each(&mut per_row, |(y, counts)| {
        let y = *y;
        let columns: Vec<u32> = (0..region.width).step_by(step as usize).collect();
        let mut stack = Vec::new();
        let rgba: Vec<f32> = columns
            .iter()
            .flat_map(|&x| compositor.pixel(region.x + x, y, &mut stack))
            .collect();
        let mut colors = vec![[0f32; 4]; columns.len()];
        sampler.convert(&rgba, &mut colors);
        for (&x, &[r, g, b, alpha]) in columns.iter().zip(&colors) {
            if alpha <= 0.0 {
                continue;
            }
            let weight = match &mask {
                Some((image, codec)) => {
                    f64::from(MaskReader { image, codec }.at(f64::from(region.x + x), f64::from(y)))
                }
                None => 1.0,
            };
            if weight <= 0.0 {
                continue;
            }
            counts.red[r as usize] += weight;
            counts.green[g as usize] += weight;
            counts.blue[b as usize] += weight;
            counts.luminosity[luminosity(r, g, b)] += weight;
        }
    });
    let mut total = Histogram::empty(step);
    for (_, counts) in &per_row {
        total.add(counts);
    }
    total
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::blend::BlendMode;
    use crate::color::PixelFormat;
    use crate::document::{Layer, LayerContent};
    use crate::edit::Edit;
    use crate::geom::Size;
    use crate::raster::RasterImage;
    use crate::selection::Selection;

    /// A document of one opaque 8-bit layer, its left half black and its right half the sRGB
    /// gray `right`.
    fn half_and_half(size: Size, right: u8) -> Document {
        let mut doc = Document::new(size);
        let width = size.width as usize;
        let pixels: Vec<u8> = (0..size.pixel_count() as usize)
            .flat_map(|i| {
                let v = if i % width >= width / 2 { right } else { 0 };
                [v, v, v, 255]
            })
            .collect();
        let image = RasterImage::from_pixels(size, PixelFormat::RGBA8_SRGB, &pixels).unwrap();
        let layer = Layer {
            style: None,
            id: doc.allocate_layer_id(),
            name: "image".into(),
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask: None,
            clipped: false,
            transform: crate::transform::Projective::IDENTITY,
            content: LayerContent::raster(Arc::new(image)),
        };
        Edit::InsertLayer {
            parent: None,
            index: 0,
            layer,
        }
        .apply(&mut doc)
        .unwrap();
        doc
    }

    #[test]
    fn counts_every_pixel_of_a_small_image_by_its_displayed_value() {
        let h = histogram(&half_and_half(Size::new(8, 4), 200));
        assert_eq!(h.step, 1);
        for channel in [&h.red, &h.green, &h.blue, &h.luminosity] {
            assert_eq!(channel[0], 16.0);
            assert_eq!(channel[200], 16.0);
            assert_eq!(channel.iter().sum::<f64>(), 32.0);
        }
    }

    #[test]
    fn a_large_image_is_sampled_and_keeps_its_proportions() {
        let h = histogram(&half_and_half(Size::new(2048, 512), 255));
        assert!(h.step > 1);
        let total: f64 = h.red.iter().sum();
        assert!(total <= MAX_SAMPLES as f64);
        assert!((h.red[255] / total - 0.5).abs() < 0.01);
    }

    #[test]
    fn the_selection_limits_it() {
        let size = Size::new(8, 4);
        let mut doc = half_and_half(size, 255);
        // The right half selected.
        use crate::selection::{Combine, EdgeOptions, Shape, select_shape};
        let shape = Shape::Rectangle {
            left: 4.0,
            top: 0.0,
            right: 8.0,
            bottom: 4.0,
        };
        let image = select_shape(size, None, &shape, EdgeOptions::default(), Combine::Replace)
            .unwrap()
            .unwrap();
        let selection = Selection::new(Arc::new(image)).unwrap();
        Edit::SetSelection {
            selection: Some(selection),
        }
        .apply(&mut doc)
        .unwrap();
        let h = histogram(&doc);
        assert_eq!(h.luminosity[255], 16.0);
        assert_eq!(h.luminosity.iter().sum::<f64>(), 16.0);
    }

    #[test]
    fn transparent_pixels_and_an_empty_document_count_nothing() {
        let h = histogram(&Document::new(Size::new(4, 4)));
        assert_eq!(h.luminosity.iter().sum::<f64>(), 0.0);
    }

    /// Timing of the histogram of a 12 MP image under a Curves layer (run with `--ignored
    /// --nocapture`).
    #[test]
    #[ignore = "benchmark"]
    fn bench_histogram() {
        let mut doc = half_and_half(Size::new(4000, 3000), 200);
        let curve = crate::curve::Curve::new(&[[0, 0], [90, 140], [255, 255]]).unwrap();
        let curves = crate::adjust::Adjustment::Curves {
            rgb: curve,
            red: crate::curve::Curve::IDENTITY,
            green: curve,
            blue: crate::curve::Curve::IDENTITY,
        };
        let layer = Layer {
            style: None,
            id: doc.allocate_layer_id(),
            name: "curves".into(),
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask: None,
            clipped: false,
            transform: crate::transform::Projective::IDENTITY,
            content: LayerContent::Adjustment { adjustment: curves },
        };
        Edit::InsertLayer {
            parent: None,
            index: 1,
            layer,
        }
        .apply(&mut doc)
        .unwrap();
        histogram(&doc);
        let start = std::time::Instant::now();
        for _ in 0..10 {
            histogram(&doc);
        }
        let each = start.elapsed().as_secs_f64() * 100.0;
        println!("histogram of 12 MP under Curves: {each:.2} ms");
    }
}
