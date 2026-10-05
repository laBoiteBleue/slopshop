//! Image > Auto Tone, Auto Contrast and Auto Color: the visible image analyzed once, the result
//! a Levels adjustment with the computed settings (applied as an effect of the layers' stacks,
//! ADR 0029). Photoshop's three classic algorithms (its "Auto Color Correction Options"), each
//! clipping 0.1 % of the pixels at both ends:
//!
//! - **Auto Contrast** (enhance monochromatic contrast): the same input black and white for the
//!   three channels, so colors keep their relations.
//! - **Auto Tone** (enhance per channel contrast): each channel stretched on its own, which can
//!   remove or add a cast.
//! - **Auto Color** (find dark and light colors, snap neutral midtones): each channel mapped
//!   from the average of the darkest pixels to the average of the lightest, then its gamma set
//!   so that the average nearly neutral midtone becomes gray.
//!
//! The analysis reads the image as composited, in the document's blend space (where Levels
//! apply), within the selection when there is one (weighted by its coverage and by the pixels'
//! alpha). Large images are sampled on a regular grid of rows and columns, up to
//! [`MAX_SAMPLES`] pixels: percentiles and averages need no more.

use crate::adjust::{Adjustment, LEVELS_IDENTITY, LevelsChannel};
use crate::blend::Blender;
use crate::composite::{CompositeError, composite_region_serial};
use crate::document::Document;
use crate::geom::Rect;
use crate::paint::MaskReader;
use crate::raster::{Codec, parallel_for_each};

/// Which automatic correction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoKind {
    Tone,
    Contrast,
    Color,
}

/// Pixels analyzed at most.
pub const MAX_SAMPLES: u64 = 1 << 21;

/// The share of the pixels clipped at each end (Photoshop's default, 0.1 %).
const CLIP: f64 = 0.001;

/// Histogram bins over `[0, 1]`.
const BINS: usize = 4096;

/// The narrowest input range a channel is stretched from (2 of 255, as the Levels dialog).
const MIN_RANGE: f64 = 2.0 / 255.0;

/// A pixel analyzed: its straight color in the blend space and its weight.
#[derive(Debug, Clone, Copy)]
struct Sample {
    color: [f64; 3],
    weight: f64,
}

/// The Levels `kind` computes for the visible image of `document`; `None` when there is nothing
/// to analyze (no pixel shows) or nothing to change.
pub fn auto_levels(
    document: &Document,
    kind: AutoKind,
) -> Result<Option<Adjustment>, CompositeError> {
    let samples = sample(document)?;
    if samples.iter().map(|s| s.weight).sum::<f64>() <= 0.0 {
        return Ok(None);
    }
    let levels = match kind {
        AutoKind::Contrast => contrast(&samples),
        AutoKind::Tone => tone(&samples),
        AutoKind::Color => color(&samples),
    };
    Ok(levels.filter(|l| l.is_valid()))
}

/// The pixels to analyze: the canvas, or the selection's bounds, on a grid of at most
/// [`MAX_SAMPLES`].
fn sample(document: &Document) -> Result<Vec<Sample>, CompositeError> {
    let size = document.size();
    let canvas = size.bounds();
    let selection = document.selection();
    let region = match selection {
        Some(s) => match crate::selection::bounds(s.image()).and_then(|b| b.intersection(canvas)) {
            Some(r) => r,
            None => return Ok(Vec::new()),
        },
        None => canvas,
    };
    if region.is_empty() {
        return Ok(Vec::new());
    }
    let pixels = region.size().pixel_count();
    let step = (pixels.div_ceil(MAX_SAMPLES) as f64).sqrt().ceil().max(1.0) as u32;
    let rows: Vec<u32> = (region.y..region.y + region.height)
        .step_by(step as usize)
        .collect();
    let blender = Blender::new(document.blend_space());
    let mask = selection.map(|s| (s.image().as_ref(), Codec::new(s.image().stored_format())));
    let mut per_row: Vec<Result<Vec<Sample>, CompositeError>> =
        (0..rows.len()).map(|_| Ok(Vec::new())).collect();
    let mut jobs: Vec<(u32, &mut Result<Vec<Sample>, CompositeError>)> =
        rows.iter().copied().zip(per_row.iter_mut()).collect();
    parallel_for_each(&mut jobs, |(y, out)| {
        let y = *y;
        let mut values = vec![0.0f32; region.width as usize * 4];
        if let Err(e) = composite_region_serial(
            document,
            Rect::new(region.x, y, region.width, 1),
            &mut values,
        ) {
            **out = Err(e);
            return;
        }
        let mut row = Vec::new();
        for x in (0..region.width).step_by(step as usize) {
            let px = &values[x as usize * 4..][..4];
            let alpha = f64::from(px[3]);
            if alpha <= 0.0 {
                continue;
            }
            let coverage = match &mask {
                Some((image, codec)) => {
                    f64::from(MaskReader { image, codec }.at(f64::from(region.x + x), f64::from(y)))
                }
                None => 1.0,
            };
            if coverage <= 0.0 {
                continue;
            }
            let encoded =
                blender.encode_premultiplied(&[px[0], px[1], px[2], px[3]].map(f64::from));
            row.push(Sample {
                color: [0, 1, 2].map(|c| encoded[c] / alpha),
                weight: alpha.min(1.0) * coverage,
            });
        }
        **out = Ok(row);
    });
    drop(jobs);
    let mut samples = Vec::new();
    for row in per_row {
        samples.extend(row?);
    }
    Ok(samples)
}

/// A weighted histogram over `[0, 1]` (values outside count at the ends).
struct Histogram {
    bins: Vec<f64>,
    total: f64,
}

impl Histogram {
    fn new() -> Self {
        Self {
            bins: vec![0.0; BINS],
            total: 0.0,
        }
    }

    fn add(&mut self, v: f64, weight: f64) {
        let bin = ((v.clamp(0.0, 1.0) * BINS as f64) as usize).min(BINS - 1);
        self.bins[bin] += weight;
        self.total += weight;
    }

    /// The value below which `share` of the weight lies: the low edge of the bin where it is
    /// reached from the bottom, or the high edge from the top.
    fn low(&self, share: f64) -> f64 {
        let target = self.total * share;
        let mut sum = 0.0;
        for (i, w) in self.bins.iter().enumerate() {
            sum += w;
            if sum > target {
                return i as f64 / BINS as f64;
            }
        }
        1.0
    }

    fn high(&self, share: f64) -> f64 {
        let target = self.total * share;
        let mut sum = 0.0;
        for (i, w) in self.bins.iter().enumerate().rev() {
            sum += w;
            if sum > target {
                return (i + 1) as f64 / BINS as f64;
            }
        }
        0.0
    }
}

/// Input black and white stretched to the whole range, or `None` when the range is too narrow
/// (a flat channel) or already whole.
fn stretch(low: f64, high: f64) -> Option<LevelsChannel> {
    if high - low < MIN_RANGE || (low <= 0.0 && high >= 1.0) {
        return None;
    }
    Some([low as f32, high as f32, 1.0, 0.0, 1.0])
}

fn levels(composite: LevelsChannel, channels: [LevelsChannel; 3]) -> Option<Adjustment> {
    let [input_black, input_white, gamma, output_black, output_white] = composite;
    if composite == LEVELS_IDENTITY && channels == [LEVELS_IDENTITY; 3] {
        return None;
    }
    Some(Adjustment::Levels {
        input_black,
        input_white,
        gamma,
        output_black,
        output_white,
        channels,
    })
}

fn contrast(samples: &[Sample]) -> Option<Adjustment> {
    let mut all = Histogram::new();
    for s in samples {
        for v in s.color {
            all.add(v, s.weight);
        }
    }
    let composite = stretch(all.low(CLIP), all.high(CLIP))?;
    levels(composite, [LEVELS_IDENTITY; 3])
}

fn tone(samples: &[Sample]) -> Option<Adjustment> {
    let mut channels = [Histogram::new(), Histogram::new(), Histogram::new()];
    for s in samples {
        for (h, v) in channels.iter_mut().zip(s.color) {
            h.add(v, s.weight);
        }
    }
    let channels = channels.map(|h| stretch(h.low(CLIP), h.high(CLIP)).unwrap_or(LEVELS_IDENTITY));
    levels(LEVELS_IDENTITY, channels)
}

/// Luma of a blend-space color (Rec. 709 weights, as Photoshop's luminosity).
fn luma(c: [f64; 3]) -> f64 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn color(samples: &[Sample]) -> Option<Adjustment> {
    // The darkest and the lightest pixels (by luma, 0.1 % each): their average colors.
    let mut lumas = Histogram::new();
    for s in samples {
        lumas.add(luma(s.color), s.weight);
    }
    let (dark_luma, light_luma) = (lumas.low(CLIP), lumas.high(CLIP));
    let average = |keep: &dyn Fn(f64) -> bool| {
        let (mut sum, mut weight) = ([0.0; 3], 0.0);
        for s in samples.iter().filter(|s| keep(luma(s.color))) {
            for (a, v) in sum.iter_mut().zip(s.color) {
                *a += v.clamp(0.0, 1.0) * s.weight;
            }
            weight += s.weight;
        }
        (weight > 0.0).then(|| sum.map(|a| a / weight))
    };
    let dark = average(&|l| l <= dark_luma + 1.0 / BINS as f64)?;
    let light = average(&|l| l >= light_luma - 1.0 / BINS as f64)?;
    let mut channels = [0, 1, 2].map(|c| stretch(dark[c], light[c]).unwrap_or(LEVELS_IDENTITY));

    // Snap neutral midtones: the average of the nearly gray midtones, once mapped, made gray by
    // each channel's gamma.
    let map = |c: usize, v: f64| {
        let [ib, iw, ..] = channels[c].map(f64::from);
        ((v - ib) / (iw - ib)).clamp(0.0, 1.0)
    };
    let (mut sum, mut weight) = ([0.0; 3], 0.0);
    for s in samples {
        let m = [0, 1, 2].map(|c| map(c, s.color[c]));
        let chroma = m[0].max(m[1]).max(m[2]) - m[0].min(m[1]).min(m[2]);
        let l = luma(m);
        if chroma < NEUTRAL_CHROMA && (0.2..=0.8).contains(&l) {
            for (a, v) in sum.iter_mut().zip(m) {
                *a += v * s.weight;
            }
            weight += s.weight;
        }
    }
    let total: f64 = samples.iter().map(|s| s.weight).sum();
    if weight > total * CLIP {
        let midtone = sum.map(|a| a / weight);
        let gray = (midtone[0] + midtone[1] + midtone[2]) / 3.0;
        for (channel, m) in channels.iter_mut().zip(midtone) {
            // m^(1/γ) = gray.
            let gamma = (m.ln() / gray.ln()).clamp(0.1, 9.99);
            if gamma.is_finite() && (gamma - 1.0).abs() > 1e-3 {
                channel[2] = gamma as f32;
            }
        }
    }
    levels(LEVELS_IDENTITY, channels)
}

/// How far from gray (the spread of the mapped channels) a midtone may be to count as neutral.
const NEUTRAL_CHROMA: f64 = 0.1;

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::blend::{BlendMode, BlendSpace};
    use crate::color::PixelFormat;
    use crate::document::{Layer, LayerContent};
    use crate::edit::Edit;
    use crate::geom::Size;
    use crate::raster::RasterImage;

    /// A linear-blending document of one 8-bit layer whose pixel `i` is `pixel(i)` (sRGB values
    /// 0–255, opaque).
    fn document(size: Size, space: BlendSpace, pixel: impl Fn(usize) -> [u8; 3]) -> Document {
        let mut doc = Document::new(size);
        Edit::SetBlendSpace { space }.apply(&mut doc).unwrap();
        let pixels: Vec<u8> = (0..size.pixel_count() as usize)
            .flat_map(|i| {
                let [r, g, b] = pixel(i);
                [r, g, b, 255]
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

    fn settings(a: Option<Adjustment>) -> (LevelsChannel, [LevelsChannel; 3]) {
        match a {
            Some(Adjustment::Levels {
                input_black,
                input_white,
                gamma,
                output_black,
                output_white,
                channels,
            }) => (
                [input_black, input_white, gamma, output_black, output_white],
                channels,
            ),
            other => panic!("not Levels: {other:?}"),
        }
    }

    fn near(a: f32, b: f64) -> bool {
        (f64::from(a) - b).abs() < 2.0 / BINS as f64 + 1e-6
    }

    /// A gradient from 50 to 200 in red, 100 to 150 in green, constant 120 in blue (perceptual
    /// blending reads sRGB values as they are).
    fn gradient() -> Document {
        document(Size::new(151, 4), BlendSpace::Perceptual, |i| {
            let x = (i % 151) as u8;
            [50 + x, 100 + x / 3, 120]
        })
    }

    #[test]
    fn auto_contrast_stretches_all_channels_alike() {
        let (composite, channels) = settings(auto_levels(&gradient(), AutoKind::Contrast).unwrap());
        assert_eq!(channels, [LEVELS_IDENTITY; 3]);
        // The darkest value of any channel to the lightest (0.1 % of 1812 values: one clipped).
        assert!(near(composite[0], 50.0 / 255.0), "{composite:?}");
        assert!(near(composite[1], 201.0 / 255.0) || near(composite[1], 200.0 / 255.0));
        assert_eq!(composite[2..], [1.0, 0.0, 1.0]);
    }

    #[test]
    fn auto_tone_stretches_each_channel_and_leaves_a_flat_one() {
        let (composite, channels) = settings(auto_levels(&gradient(), AutoKind::Tone).unwrap());
        assert_eq!(composite, LEVELS_IDENTITY);
        assert!(near(channels[0][0], 50.0 / 255.0) && channels[0][1] > channels[1][1]);
        assert!(near(channels[1][0], 100.0 / 255.0), "{channels:?}");
        // Blue is flat: nothing to stretch.
        assert_eq!(channels[2], LEVELS_IDENTITY);
    }

    #[test]
    fn auto_color_maps_dark_and_light_colors_and_neutralizes_the_midtones() {
        // A cast: blue a little dark everywhere, from a dark to a light end.
        let doc = document(Size::new(201, 3), BlendSpace::Perceptual, |i| {
            let x = (i % 201) as f64 / 200.0;
            let v = |lo: f64, hi: f64| (lo + (hi - lo) * x).round() as u8;
            [v(20.0, 235.0), v(20.0, 235.0), v(10.0, 200.0)]
        });
        let (composite, channels) = settings(auto_levels(&doc, AutoKind::Color).unwrap());
        assert_eq!(composite, LEVELS_IDENTITY);
        // Each channel from its dark end to its light end.
        assert!(
            (f64::from(channels[2][0]) - 10.0 / 255.0).abs() < 0.01,
            "{channels:?}"
        );
        assert!(
            (f64::from(channels[2][1]) - 200.0 / 255.0).abs() < 0.01,
            "{channels:?}"
        );
        assert!(
            (f64::from(channels[0][1]) - 235.0 / 255.0).abs() < 0.01,
            "{channels:?}"
        );
        // Once applied, a midtone is gray.
        let adjustment = auto_levels(&doc, AutoKind::Color).unwrap().unwrap();
        let mid = [127.5 / 255.0, 127.5 / 255.0, 105.0 / 255.0];
        let [r, g, b] = adjustment.apply(mid);
        assert!((r - g).abs() < 0.02 && (g - b).abs() < 0.02, "{r} {g} {b}");
    }

    #[test]
    fn nothing_to_analyze_or_to_change_gives_nothing() {
        let empty = Document::new(Size::new(10, 10));
        assert_eq!(auto_levels(&empty, AutoKind::Tone), Ok(None));
        // Already from black to white in every channel.
        let full = document(Size::new(256, 1), BlendSpace::Perceptual, |i| [i as u8; 3]);
        for kind in [AutoKind::Tone, AutoKind::Contrast] {
            assert_eq!(auto_levels(&full, kind), Ok(None), "{kind:?}");
        }
    }

    #[test]
    fn the_selection_limits_what_is_analyzed() {
        let mut doc = gradient();
        // Only the 51 darkest columns (red 50–100, green 100–116).
        let size = doc.size();
        let pixels: Vec<u8> = (0..size.pixel_count() as u32)
            .flat_map(|i| {
                let v: u16 = if i % size.width < 51 { u16::MAX } else { 0 };
                v.to_ne_bytes()
            })
            .collect();
        let image =
            RasterImage::from_pixels(size, crate::selection::SELECTION_FORMAT, &pixels).unwrap();
        let selection = crate::selection::Selection::new(Arc::new(image)).unwrap();
        Edit::SetSelection {
            selection: Some(selection),
        }
        .apply(&mut doc)
        .unwrap();
        let (_, channels) = settings(auto_levels(&doc, AutoKind::Tone).unwrap());
        assert!(near(channels[0][1], 101.0 / 255.0) || near(channels[0][1], 100.0 / 255.0));
    }

    #[test]
    fn large_images_are_sampled() {
        let size = Size::new(3000, 2000);
        let doc = document(size, BlendSpace::Perceptual, |i| {
            let x = (i % 3000) as f64 / 2999.0;
            [(30.0 + 180.0 * x) as u8; 3]
        });
        assert!(sample(&doc).unwrap().len() as u64 <= MAX_SAMPLES);
        let (composite, _) = settings(auto_levels(&doc, AutoKind::Contrast).unwrap());
        assert!((f64::from(composite[0]) - 30.0 / 255.0).abs() < 0.01);
        assert!((f64::from(composite[1]) - 210.0 / 255.0).abs() < 0.01);
    }
}
