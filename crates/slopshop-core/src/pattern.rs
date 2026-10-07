//! Patterns (ADR 0042): an image source repeated across the plane. A pattern fill layer shows
//! one, scaled and turned in its content space; the compositor and the renderer sample it as a
//! transformed raster is sampled, its texel coordinates wrapping around the image.

use std::sync::Arc;

use crate::raster::RasterImage;
use crate::source::Source;
use crate::transform::Affine;

/// The smallest and largest scale of a pattern (Photoshop's 1 % to 1000 %).
pub const MIN_SCALE: f64 = 0.01;
pub const MAX_SCALE: f64 = 10.0;

/// What a pattern fill layer shows.
#[derive(Debug, Clone)]
pub struct PatternFill {
    /// The image repeated, kept once (ADR 0040).
    pub source: Arc<Source>,
    /// Layer content pixels per pattern pixel (1: 100 %).
    pub scale: f64,
    /// Degrees, counterclockwise on screen.
    pub angle: f64,
}

/// The same source (by identity), scale and angle.
impl PartialEq for PatternFill {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.source, &other.source)
            && self.scale == other.scale
            && self.angle == other.angle
    }
}

impl PatternFill {
    /// A pattern of `source` at 100 %, upright.
    pub fn new(source: Arc<Source>) -> Self {
        Self {
            source,
            scale: 1.0,
            angle: 0.0,
        }
    }

    pub fn is_valid(&self) -> bool {
        (MIN_SCALE..=MAX_SCALE).contains(&self.scale) && self.angle.is_finite()
    }

    /// Pattern pixels → the layer's content space: scaled, then turned about the origin.
    pub fn to_content(&self) -> Affine {
        // `Affine::rotation` turns clockwise on screen.
        Affine::scale(self.scale, self.scale).then(Affine::rotation(-self.angle.to_radians()))
    }
}

/// How many pyramid levels of `image` a pattern samples: those whose size divides the image's
/// exactly, so that each repeats with the same period (a coarser level of an odd size would
/// drift by a pixel each period: seams). At least one.
pub fn exact_levels(image: &RasterImage) -> usize {
    let size = image.size();
    let available = image.levels().len().max(1);
    let mut levels = 1;
    while levels < available {
        let factor = 1u32 << levels;
        if !size.width.is_multiple_of(factor) || !size.height.is_multiple_of(factor) {
            break;
        }
        levels += 1;
    }
    levels
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::PixelFormat;
    use crate::geom::Size;

    fn image(width: u32, height: u32) -> Arc<RasterImage> {
        let size = Size::new(width, height);
        let bytes = vec![128u8; size.pixel_count() as usize * 4];
        Arc::new(RasterImage::from_pixels(size, PixelFormat::RGBA8_SRGB, &bytes).unwrap())
    }

    #[test]
    fn a_pattern_samples_the_levels_that_repeat_exactly() {
        // 3000 × 1000: 1500 × 500, 750 × 250, 375 × 125, then 187.5: four levels of five.
        let even = image(3000, 1000);
        assert!(even.levels().len() > 4);
        assert_eq!(exact_levels(&even), 4);
        // An odd size: the full size only.
        assert_eq!(exact_levels(&image(1001, 1024)), 1);
    }

    #[test]
    fn a_pattern_is_scaled_then_turned_counterclockwise() {
        let source = Source::new(image(8, 8), "dots");
        let mut fill = PatternFill::new(source);
        fill.scale = 2.0;
        fill.angle = 90.0;
        // (1, 0) scaled to (2, 0), then a quarter turn counterclockwise on screen: up.
        let (x, y) = fill.to_content().apply(1.0, 0.0);
        assert!(x.abs() < 1e-12 && (y + 2.0).abs() < 1e-12, "({x}, {y})");
        assert!(fill.is_valid());
        fill.scale = 0.0;
        assert!(!fill.is_valid());
    }

    use crate::blend::{BlendMode, BlendSpace};
    use crate::composite::composite_region;
    use crate::document::{Document, Layer, LayerContent, LayerId};
    use crate::edit::{Edit, EditError};
    use crate::geom::Rect;
    use crate::transform::Projective;

    /// An 8 × 8 pattern, each texel its own color (linear sRGB, opaque).
    fn numbered() -> Arc<RasterImage> {
        let size = Size::new(8, 8);
        let mut bytes = Vec::new();
        for y in 0..8u8 {
            for x in 0..8u8 {
                bytes.extend([x * 30, y * 30, 100, 255]);
            }
        }
        let format = PixelFormat {
            color_space: crate::color::ColorSpace::LINEAR_SRGB,
            ..PixelFormat::RGBA8_SRGB
        };
        Arc::new(RasterImage::from_pixels(size, format, &bytes).unwrap())
    }

    fn with_pattern(pattern: PatternFill, transform: Projective) -> (Document, LayerId) {
        let mut doc = Document::new(Size::new(40, 30));
        Edit::SetBlendSpace {
            space: BlendSpace::Linear,
        }
        .apply(&mut doc)
        .unwrap();
        let id = doc.allocate_layer_id();
        Edit::InsertLayer {
            parent: None,
            index: 0,
            layer: Layer {
                id,
                name: "Pattern 1".into(),
                visible: true,
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                content: LayerContent::PatternFill { pattern },
                mask: None,
                clipped: false,
                transform,
                style: None,
            },
        }
        .apply(&mut doc)
        .unwrap();
        (doc, id)
    }

    fn pixels(doc: &Document) -> Vec<f32> {
        let mut out = vec![0.0f32; 40 * 30 * 4];
        composite_region(doc, Rect::new(0, 0, 40, 30), &mut out).unwrap();
        out
    }

    fn at(out: &[f32], x: usize, y: usize) -> [f32; 4] {
        let i = (y * 40 + x) * 4;
        [out[i], out[i + 1], out[i + 2], out[i + 3]]
    }

    #[test]
    fn a_pattern_repeats_across_the_canvas_from_the_layers_origin() {
        let source = Source::new(numbered(), "numbers");
        let (doc, id) = with_pattern(PatternFill::new(Arc::clone(&source)), Projective::IDENTITY);
        let out = pixels(&doc);
        // The image itself, as a pixel layer in its place, for reference.
        let mut plain = doc.clone();
        Edit::RemoveLayer { id }.apply(&mut plain).unwrap();
        let raster = plain.allocate_layer_id();
        Edit::InsertLayer {
            parent: None,
            index: 0,
            layer: Layer {
                id: raster,
                name: "image".into(),
                visible: true,
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                content: LayerContent::raster(numbered()),
                mask: None,
                clipped: false,
                transform: Projective::IDENTITY,
                style: None,
            },
        }
        .apply(&mut plain)
        .unwrap();
        let reference = pixels(&plain);
        // Texel (x mod 8, y mod 8) everywhere, opaque.
        for (x, y) in [(0, 0), (3, 5), (8, 0), (13, 21), (39, 29)] {
            assert_eq!(at(&out, x, y), at(&reference, x % 8, y % 8), "({x}, {y})");
            assert_eq!(at(&out, x, y)[3], 1.0);
        }
        // Moved by the layer: (3, 0) shows texel (0, 0).
        let moved: Projective = crate::transform::Affine::translation(3.0, 0.0).into();
        let (doc, _) = with_pattern(PatternFill::new(source), moved);
        assert_eq!(at(&pixels(&doc), 3, 0), at(&reference, 0, 0));
    }

    #[test]
    fn a_scaled_and_turned_pattern_keeps_its_period() {
        let mut pattern = PatternFill::new(Source::new(numbered(), "numbers"));
        pattern.scale = 2.0;
        let (doc, _) = with_pattern(pattern.clone(), Projective::IDENTITY);
        let out = pixels(&doc);
        // A period of 16 pixels: the same pixels 16 apart, never a seam.
        for (x, y) in [(1, 1), (5, 9), (2, 13)] {
            let (a, b) = (at(&out, x, y), at(&out, x + 16, y));
            for c in 0..4 {
                assert!((a[c] - b[c]).abs() < 1e-3, "({x}, {y}): {a:?} vs {b:?}");
            }
            assert!((a[3] - 1.0).abs() < 1e-3, "opaque everywhere");
        }
        // Turned a quarter: the period goes down the canvas.
        pattern.angle = 90.0;
        let (doc, _) = with_pattern(pattern, Projective::IDENTITY);
        let out = pixels(&doc);
        let (a, b) = (at(&out, 7, 3), at(&out, 7, 19));
        for c in 0..4 {
            assert!((a[c] - b[c]).abs() < 1e-3, "{a:?} vs {b:?}");
        }
    }

    #[test]
    fn a_patterns_scale_and_angle_change_undoably_within_range() {
        let source = Source::new(numbered(), "numbers");
        let (mut doc, id) =
            with_pattern(PatternFill::new(Arc::clone(&source)), Projective::IDENTITY);
        let before = pixels(&doc);
        let mut bigger = PatternFill::new(Arc::clone(&source));
        bigger.scale = 3.0;
        let undo = Edit::SetPatternFill {
            id,
            pattern: bigger,
        }
        .apply(&mut doc)
        .unwrap();
        assert_ne!(pixels(&doc), before);
        undo.apply(&mut doc).unwrap();
        assert_eq!(pixels(&doc), before);
        let mut wrong = PatternFill::new(source);
        wrong.scale = 20.0;
        assert_eq!(
            Edit::SetPatternFill { id, pattern: wrong }
                .apply(&mut doc)
                .err(),
            Some(EditError::InvalidPattern)
        );
    }

    #[test]
    fn a_pattern_shared_with_other_layers_is_made_unique_undoably() {
        let source = Source::new(numbered(), "numbers");
        let (mut doc, a) =
            with_pattern(PatternFill::new(Arc::clone(&source)), Projective::IDENTITY);
        let b = doc.allocate_layer_id();
        let mut layer = doc.layer(a).unwrap().clone();
        layer.id = b;
        Edit::InsertLayer {
            parent: None,
            index: 1,
            layer,
        }
        .apply(&mut doc)
        .unwrap();
        // The document's sources list the pattern, shown by both layers.
        let sources = doc.sources();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].1.len(), 2);
        let source_of = |doc: &Document, id| match &doc.layer(id).unwrap().content {
            LayerContent::PatternFill { pattern } => Arc::clone(&pattern.source),
            _ => unreachable!("a pattern fill"),
        };
        let undo = Edit::make_unique(&doc, &[b])
            .unwrap()
            .apply(&mut doc)
            .unwrap();
        assert!(!Arc::ptr_eq(&source_of(&doc, a), &source_of(&doc, b)));
        assert!(Arc::ptr_eq(source_of(&doc, b).image(), source.image()));
        undo.apply(&mut doc).unwrap();
        assert!(Arc::ptr_eq(&source_of(&doc, a), &source_of(&doc, b)));
    }
}
