//! Text layout for SlopShop's text layers (ADR 0041): a [`Text`] laid out by `parley` (bidi,
//! line breaks, shaping through `harfrust`, font fallback through `fontique`) into glyph
//! outlines read by `skrifa`, unhinted. The outlines are a [`Shape`] filled with the text's
//! color, kept in the text's source and drawn through the shapes' coverage, so that the CPU and
//! the GPU agree by construction and a file shows the same text on every computer.
//!
//! Fonts: the system's (listed for the font picker) and Inter, bundled (SIL Open Font License,
//! `fonts/Inter-LICENSE.txt`) as the default family, the fallback for a missing family and the
//! only font tests use, so that they do not depend on the machine.

use std::sync::Arc;

use fontique::{Blob, Collection, CollectionOptions, FontInfo, SourceCache};
use parley::{
    Affinity, Alignment, AlignmentOptions, BreakReason, Cursor, FontContext, FontFamily, FontStyle,
    FontWeight, Layout, LayoutContext, LineHeight, PositionedLayoutItem, StyleProperty,
};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::prelude::{LocationRef, Size};
use skrifa::raw::types::F2Dot14;
use skrifa::{GlyphId, MetadataProvider};
use slopshop_core::shape::{FillRule, Geometry, Paint, Segment, Shape, Subpath, Text, TextAlign};

/// The bundled default family's name.
pub const DEFAULT_FAMILY: &str = "Inter Variable";

/// Inter (OFL-1.1), upright and italic, variable in weight.
const INTER: &[u8] = include_bytes!("../fonts/InterVariable.ttf");
const INTER_ITALIC: &[u8] = include_bytes!("../fonts/InterVariable-Italic.ttf");

/// The bundled font's license, to be shown with the application's notices.
pub const INTER_LICENSE: &str = include_str!("../fonts/Inter-LICENSE.txt");

/// One style a family offers, for the picker.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontStyleInfo {
    pub weight: f32,
    pub italic: bool,
}

/// A caret: the top and bottom of the line it stands in, at a place in the text, in the layer's
/// own space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Caret {
    pub top: [f64; 2],
    pub bottom: [f64; 2],
}

/// A line of a laid out text, in the layer's own space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineBox {
    /// Its first place and the place after its last character, in UTF-16 code units: a line
    /// ended by a line break holds it.
    pub start: usize,
    pub end: usize,
    /// It ends at a line break (`\n`), not where the text wrapped.
    pub hard_break: bool,
    /// Its box: from the start to the end of its content, from the top to the bottom of its
    /// line box.
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
}

/// Where a caret can stand in a text and its lines, for editing it: the carets, one per place
/// counted in UTF-16 code units (the interface's strings), `content.encode_utf16().count() + 1`
/// of them, a place inside a character given the caret before it; the lines in order.
#[derive(Debug, Clone, PartialEq)]
pub struct TextGeometry {
    pub carets: Vec<Caret>,
    pub lines: Vec<LineBox>,
}

/// What laying out a text gives.
#[derive(Debug, Clone, PartialEq)]
pub struct Laid {
    /// The glyph outlines, filled by the text's color, in the layer's own space.
    pub outlines: Shape,
    /// The family was not found: the text was set in the default family.
    pub missing_font: bool,
}

/// The fonts known to the application, and what lays text out with them. Not shared between
/// threads: callers keep one behind a lock.
pub struct Fonts {
    fonts: FontContext,
    layout: LayoutContext<()>,
}

impl std::fmt::Debug for Fonts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fonts").finish_non_exhaustive()
    }
}

/// The weights a picker lists for a variable font (Photoshop's names, Thin to Black).
const NAMED_WEIGHTS: [f32; 9] = [
    100.0, 200.0, 300.0, 400.0, 500.0, 600.0, 700.0, 800.0, 900.0,
];

impl Fonts {
    /// The system's fonts and the bundled ones.
    pub fn system() -> Self {
        Self::with(true)
    }

    /// The bundled fonts only (tests, and anything that must not depend on the machine).
    pub fn bundled() -> Self {
        Self::with(false)
    }

    fn with(system_fonts: bool) -> Self {
        let mut collection = Collection::new(CollectionOptions {
            shared: false,
            system_fonts,
        });
        for data in [INTER, INTER_ITALIC] {
            collection.register_fonts(Blob::new(Arc::new(data)), None);
        }
        Self {
            fonts: FontContext {
                collection,
                source_cache: SourceCache::default(),
            },
            layout: LayoutContext::new(),
        }
    }

    /// The families to choose from, sorted, without duplicates.
    pub fn families(&mut self) -> Vec<String> {
        let mut names: Vec<String> = self
            .fonts
            .collection
            .family_names()
            .map(str::to_owned)
            .collect();
        names.sort_by_key(|n| n.to_lowercase());
        names.dedup();
        names
    }

    /// Whether `family` is known.
    pub fn has_family(&mut self, family: &str) -> bool {
        self.fonts.collection.family_id(family).is_some()
    }

    /// The styles `family` offers, upright first, lightest first: a variable font's named
    /// weights within its range. Empty for an unknown family.
    pub fn styles(&mut self, family: &str) -> Vec<FontStyleInfo> {
        let Some(id) = self.fonts.collection.family_id(family) else {
            return Vec::new();
        };
        let Some(info) = self.fonts.collection.family(id) else {
            return Vec::new();
        };
        let mut styles = Vec::new();
        for font in info.fonts() {
            let italic = font.style() != FontStyle::Normal;
            match weight_axis(font) {
                Some((min, max)) => styles.extend(
                    NAMED_WEIGHTS
                        .iter()
                        .filter(|&&w| (min..=max).contains(&w))
                        .map(|&weight| FontStyleInfo { weight, italic }),
                ),
                None => styles.push(FontStyleInfo {
                    weight: font.weight().value(),
                    italic,
                }),
            }
        }
        styles.sort_by(|a, b| {
            (a.italic, a.weight)
                .partial_cmp(&(b.italic, b.weight))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        styles.dedup();
        styles
    }

    fn build(&mut self, text: &Text) -> (Layout<()>, bool) {
        let missing_font = !self.has_family(&text.family);
        let family = if missing_font {
            DEFAULT_FAMILY
        } else {
            text.family.as_str()
        };
        let mut builder = self
            .layout
            .ranged_builder(&mut self.fonts, &text.content, 1.0, false);
        builder.push_default(StyleProperty::FontFamily(FontFamily::named(family)));
        builder.push_default(StyleProperty::FontSize(text.size as f32));
        builder.push_default(StyleProperty::FontWeight(FontWeight::new(text.weight)));
        builder.push_default(StyleProperty::FontStyle(if text.italic {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        }));
        builder.push_default(StyleProperty::LineHeight(LineHeight::Absolute(
            text.line_spacing() as f32,
        )));
        builder.push_default(StyleProperty::LetterSpacing(
            (text.tracking / 1000.0 * text.size) as f32,
        ));
        let mut layout: Layout<()> = builder.build(&text.content);
        layout.break_all_lines(None);
        let alignment = match text.align {
            TextAlign::Left => Alignment::Left,
            TextAlign::Center => Alignment::Center,
            TextAlign::Right => Alignment::Right,
        };
        layout.align(alignment, AlignmentOptions::default());
        (layout, missing_font)
    }

    /// Where the layout's origin (its top left) lies in the layer's space: the anchor is the
    /// first baseline at the alignment point.
    fn origin(text: &Text, layout: &Layout<()>) -> [f64; 2] {
        let width = f64::from(layout.width());
        let dx = match text.align {
            TextAlign::Left => 0.0,
            TextAlign::Center => width / 2.0,
            TextAlign::Right => width,
        };
        let baseline = f64::from(layout.first_baseline().unwrap_or(0.0));
        [text.position[0] - dx, text.position[1] - baseline]
    }

    /// `text` laid out into its glyph outlines.
    pub fn lay_out(&mut self, text: &Text) -> Laid {
        let (layout, missing_font) = self.build(text);
        let [ox, oy] = Self::origin(text, &layout);
        let mut pen = Outlines::default();
        for line in layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(glyphs) = item else {
                    continue;
                };
                let run = glyphs.run();
                let font_data = run.font();
                let Ok(font) =
                    skrifa::FontRef::from_index(font_data.data.as_ref(), font_data.index)
                else {
                    continue;
                };
                let coords: Vec<F2Dot14> = run
                    .normalized_coords()
                    .iter()
                    .map(|c| F2Dot14::from_bits(c.to_bits()))
                    .collect();
                let outlines = font.outline_glyphs();
                let size = Size::new(run.font_size());
                let skew = run
                    .synthesis()
                    .skew()
                    .map_or(0.0, |deg| f64::from(deg).to_radians().tan());
                for glyph in glyphs.positioned_glyphs() {
                    let Some(outline) = outlines.get(GlyphId::new(glyph.id)) else {
                        continue;
                    };
                    pen.place = [ox + f64::from(glyph.x), oy + f64::from(glyph.y), skew];
                    let settings = DrawSettings::unhinted(size, LocationRef::new(&coords));
                    // A glyph that cannot be drawn is left out, as a missing glyph would be.
                    let _ = outline.draw(settings, &mut pen);
                    pen.finish();
                }
            }
        }
        Laid {
            outlines: Shape {
                geometry: Geometry::Path {
                    subpaths: pen.subpaths,
                    rule: FillRule::NonZero,
                },
                fill: Some(Paint::Solid(text.color)),
                stroke: None,
            },
            missing_font,
        }
    }

    /// Where a caret can stand in `text` and its lines, for editing it (see [`TextGeometry`]).
    pub fn geometry(&mut self, text: &Text) -> TextGeometry {
        let (layout, _) = self.build(text);
        let [ox, oy] = Self::origin(text, &layout);
        // UTF-16 places of byte indices: the interface's strings count in UTF-16 units.
        // parley counts a place past the end in an empty text's line: clamped.
        let utf16 = |byte: usize| {
            let byte = byte.min(text.content.len());
            text.content
                .get(..byte)
                .map_or(0, |s| s.encode_utf16().count())
        };
        let caret_at = |byte: usize| {
            let cursor = Cursor::from_byte_index(&layout, byte, Affinity::Downstream);
            let b = cursor.geometry(&layout, 1.0);
            Caret {
                top: [ox + b.x0, oy + b.y0],
                bottom: [ox + b.x0, oy + b.y1],
            }
        };
        let mut carets = Vec::with_capacity(text.content.len() + 1);
        for (byte, c) in text.content.char_indices() {
            let caret = caret_at(byte);
            carets.extend(std::iter::repeat_n(caret, c.len_utf16()));
        }
        carets.push(caret_at(text.content.len()));
        let lines = layout
            .lines()
            .map(|line| {
                let m = line.metrics();
                let range = line.text_range();
                // The line box, as parley's selection geometry takes it.
                let top = f64::from(m.block_min_coord.min(m.content_block_min_coord));
                let bottom = f64::from(m.block_max_coord.max(m.content_block_max_coord));
                let left = f64::from(m.offset);
                LineBox {
                    start: utf16(range.start),
                    end: utf16(range.end),
                    hard_break: line.break_reason() == BreakReason::Explicit,
                    left: ox + left,
                    right: ox + left + f64::from(m.advance),
                    top: oy + top,
                    bottom: oy + bottom,
                }
            })
            .collect();
        TextGeometry { carets, lines }
    }
}

/// The weight range of a variable font.
fn weight_axis(font: &FontInfo) -> Option<(f32, f32)> {
    font.axes()
        .iter()
        .find(|a| a.tag == skrifa::raw::types::Tag::new(b"wght"))
        .map(|a| (a.min, a.max))
}

/// Coordinates are kept to a thousandth of a pixel: finer is invisible and makes files larger.
fn round(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// A pen collecting glyph outlines as subpaths, placed at a glyph's origin (y down).
#[derive(Default)]
struct Outlines {
    subpaths: Vec<Subpath>,
    current: Option<Subpath>,
    last: [f64; 2],
    /// The glyph's origin, and the skew of a synthesized italic (tangent of its angle).
    place: [f64; 3],
}

impl Outlines {
    fn point(&self, x: f32, y: f32) -> [f64; 2] {
        let [ox, oy, skew] = self.place;
        let (x, y) = (f64::from(x), f64::from(y));
        [round(ox + x + y * skew), round(oy - y)]
    }

    fn finish(&mut self) {
        if let Some(subpath) = self.current.take()
            && !subpath.segments.is_empty()
        {
            self.subpaths.push(subpath);
        }
    }

    fn segment(&mut self, segment: Segment, end: [f64; 2]) {
        if let Some(current) = &mut self.current {
            current.segments.push(segment);
        }
        self.last = end;
    }
}

impl OutlinePen for Outlines {
    fn move_to(&mut self, x: f32, y: f32) {
        self.finish();
        let start = self.point(x, y);
        self.current = Some(Subpath {
            start,
            segments: Vec::new(),
            closed: true,
        });
        self.last = start;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.point(x, y);
        self.segment(Segment::Line(p), p);
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        // A quadratic as the cubic it is exactly.
        let q = self.point(cx0, cy0);
        let p = self.point(x, y);
        let l = self.last;
        let a = [
            l[0] + (q[0] - l[0]) * 2.0 / 3.0,
            l[1] + (q[1] - l[1]) * 2.0 / 3.0,
        ];
        let b = [
            p[0] + (q[0] - p[0]) * 2.0 / 3.0,
            p[1] + (q[1] - p[1]) * 2.0 / 3.0,
        ];
        self.segment(Segment::Cubic(a, b, p), p);
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        let a = self.point(cx0, cy0);
        let b = self.point(cx1, cy1);
        let p = self.point(x, y);
        self.segment(Segment::Cubic(a, b, p), p);
    }

    fn close(&mut self) {
        self.finish();
    }
}

#[cfg(test)]
mod tests;
