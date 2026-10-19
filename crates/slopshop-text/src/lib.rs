//! Text layout for SlopShop's text layers (ADR 0041): a [`Text`] laid out by `parley` (bidi,
//! line breaks, shaping through `harfrust`, font fallback through `fontique`) into glyph
//! outlines read by `skrifa`, unhinted. The outlines are a [`Shape`] filled with the text's
//! color, kept in the text's source and drawn through the shapes' coverage, so that the CPU and
//! the GPU agree by construction and a file shows the same text on every computer.
//!
//! Fonts: the system's (listed for the font picker) and Inter, bundled (SIL Open Font License,
//! `fonts/Inter-LICENSE.txt`) as the default family, the fallback for a missing family and the
//! only font tests use, so that they do not depend on the machine.

use std::borrow::Cow;
use std::ops::Range;
use std::sync::Arc;

use fontique::{Blob, Collection, CollectionOptions, FontInfo, SourceCache};
use parley::setting::{FontFeature, Tag};
use parley::{
    Affinity, Alignment, AlignmentOptions, BreakReason, Cursor, FontContext, FontFamily,
    FontFeatures, FontStyle, FontWeight, Layout, LayoutContext, LineHeight, OverflowWrap,
    PositionedLayoutItem, StyleProperty,
};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::prelude::{LocationRef, Size};
use skrifa::raw::types::F2Dot14;
use skrifa::{GlyphId, MetadataProvider};
use slopshop_core::color::LinearRgba;
use slopshop_core::shape::{
    CharStyle, FillRule, Geometry, Paint, Resolved, Segment, Shape, Subpath, Text, TextAlign,
    TextBaseline, TextCase,
};

/// Photoshop's defaults (its Type preferences): small capitals at 70 % of the size, superscript
/// and subscript at 58.3 %, raised or lowered by 33.3 % of the size.
const SMALL_CAPS_SIZE: f64 = 0.7;
const SCRIPT_SIZE: f64 = 0.583;
const SCRIPT_POSITION: f64 = 0.333;

/// Faux Bold thickens glyphs on each side by this share of their size.
const FAUX_BOLD: f64 = 1.0 / 40.0;

/// Faux Italic's slant, in degrees (as fontique slants a synthesized italic).
const FAUX_ITALIC_DEGREES: f64 = 14.0;

/// The standard ligatures turned off (Photoshop's OpenType Standard Ligatures).
static NO_LIGATURES: [FontFeature; 2] = [
    FontFeature::new(Tag::new(b"liga"), 0),
    FontFeature::new(Tag::new(b"clig"), 0),
];

/// How a run of glyphs is drawn besides its outlines (parley's brush, kept per run).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Ink {
    /// Its color's index among the text's colors, the text's own first.
    color: u16,
    /// How far its baseline is raised (superscript), or lowered when negative, in pixels.
    rise: f32,
    /// Faux Bold: how much its glyphs are thickened on each side, in pixels; 0: not.
    bold: f32,
    /// Faux Italic.
    slant: bool,
    underline: bool,
    strikethrough: bool,
}

impl Ink {
    /// How characters set in `resolved` are drawn, in the color of index `color`.
    fn of(resolved: &Resolved<'_>, color: u16) -> Self {
        let by = (resolved.size * SCRIPT_POSITION) as f32;
        Self {
            color,
            rise: match resolved.baseline {
                TextBaseline::Normal => 0.0,
                TextBaseline::Superscript => by,
                TextBaseline::Subscript => -by,
            },
            bold: 0.0,
            slant: resolved.faux_italic,
            underline: resolved.underline,
            strikethrough: resolved.strikethrough,
        }
    }
}

/// What parley sets characters in: `resolved`'s settings in `family` (an installed one), at
/// `scale` of their size (small capitals), drawn with `ink`.
fn properties<'a>(
    resolved: &Resolved<'_>,
    family: &'a str,
    scale: f64,
    ink: Ink,
) -> [StyleProperty<'a, Ink>; 8] {
    let size = font_size(resolved) * scale;
    let ink = Ink {
        bold: if resolved.faux_bold {
            (size * FAUX_BOLD) as f32
        } else {
            0.0
        },
        ..ink
    };
    [
        StyleProperty::FontFamily(FontFamily::named(family)),
        StyleProperty::FontSize(size as f32),
        StyleProperty::FontWeight(FontWeight::new(resolved.weight)),
        StyleProperty::FontStyle(if resolved.italic {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        }),
        StyleProperty::LineHeight(LineHeight::Absolute(resolved.line_spacing as f32)),
        // Faux Bold's thickening kept apart from the next glyph.
        StyleProperty::LetterSpacing((resolved.tracking / 1000.0 * size) as f32 + 2.0 * ink.bold),
        StyleProperty::FontFeatures(if resolved.ligatures {
            FontFeatures::empty()
        } else {
            (&NO_LIGATURES).into()
        }),
        StyleProperty::Brush(ink),
    ]
}

/// The size characters are set at: their own, or a superscript's or subscript's.
fn font_size(resolved: &Resolved<'_>) -> f64 {
    match resolved.baseline {
        TextBaseline::Normal => resolved.size,
        TextBaseline::Superscript | TextBaseline::Subscript => resolved.size * SCRIPT_SIZE,
    }
}

/// The text's characters in pieces set alike: its spans (their styles), and its own settings
/// between them (`None`).
fn pieces(text: &Text) -> Vec<(Range<usize>, Option<&CharStyle>)> {
    let mut pieces = Vec::with_capacity(text.spans.len() * 2 + 1);
    let mut at = 0;
    for span in &text.spans {
        if span.start > at {
            pieces.push((at..span.start, None));
        }
        pieces.push((span.start..span.end, Some(&span.style)));
        at = span.end;
    }
    if at < text.content.len() {
        pieces.push((at..text.content.len(), None));
    }
    pieces
}

/// What parley lays out: the content with All Caps and Small Caps applied (a capital may be
/// longer than its letter: "ß" gives "SS"), and where each character went.
struct Shown<'a> {
    text: Cow<'a, str>,
    /// For each byte of the content and its end, where its character starts in `text`; empty
    /// when `text` is the content.
    places: Vec<usize>,
    /// For each byte of `text` and its end, where the character it comes from starts in the
    /// content; empty when `text` is the content.
    back: Vec<usize>,
    /// The ranges of `text` set as small capitals.
    small: Vec<Range<usize>>,
}

impl<'a> Shown<'a> {
    fn new(content: &'a str, pieces: &[(Range<usize>, Resolved<'_>)]) -> Self {
        if pieces.iter().all(|(_, r)| r.case == TextCase::Normal) {
            return Self {
                text: Cow::Borrowed(content),
                places: Vec::new(),
                back: Vec::new(),
                small: Vec::new(),
            };
        }
        let mut shown = String::with_capacity(content.len());
        let mut places = vec![0; content.len() + 1];
        let mut back = Vec::with_capacity(content.len() + 1);
        let mut small: Vec<Range<usize>> = Vec::new();
        for (range, resolved) in pieces {
            for (i, c) in content[range.clone()].char_indices() {
                let at = range.start + i;
                let start = shown.len();
                places[at..at + c.len_utf8()].fill(start);
                match resolved.case {
                    TextCase::Normal => shown.push(c),
                    TextCase::AllCaps | TextCase::SmallCaps => shown.extend(c.to_uppercase()),
                }
                back.resize(shown.len(), at);
                // A small capital: a letter that has a capital (capitals stay full size).
                let changed = shown.len() - start != c.len_utf8() || !shown[start..].starts_with(c);
                if resolved.case == TextCase::SmallCaps && changed {
                    match small.last_mut() {
                        Some(last) if last.end == start => last.end = shown.len(),
                        _ => small.push(start..shown.len()),
                    }
                }
            }
        }
        places[content.len()] = shown.len();
        back.push(content.len());
        Self {
            text: Cow::Owned(shown),
            places,
            back,
            small,
        }
    }

    /// Where byte `byte` of the content is in the text laid out.
    fn shown(&self, byte: usize) -> usize {
        self.places.get(byte).copied().unwrap_or(byte)
    }

    /// Where byte `byte` of the text laid out comes from in the content.
    fn content(&self, byte: usize) -> usize {
        self.back.get(byte).copied().unwrap_or(byte)
    }
}

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
    /// Paragraph text whose last lines do not fit in its box (hidden).
    pub overflow: bool,
}

/// What laying out a text gives.
#[derive(Debug, Clone, PartialEq)]
pub struct Laid {
    /// The glyph outlines, filled by the text's color, in the layer's own space.
    pub outlines: Shape,
    /// When characters are set in other colors: the glyphs by color, each filled with it (the
    /// text's own color first), to be drawn instead of `outlines`. Empty otherwise.
    pub paints: Vec<Shape>,
    /// The family was not found: the text was set in the default family.
    pub missing_font: bool,
}

/// The fonts known to the application, and what lays text out with them. Not shared between
/// threads: callers keep one behind a lock.
pub struct Fonts {
    fonts: FontContext,
    layout: LayoutContext<Ink>,
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

    /// `text` laid out (as shown, its cases applied), whether a family was missing (set in the
    /// default family instead), and its colors: each glyph run's ink names its color's index
    /// there, the text's own first.
    fn build<'t>(&mut self, text: &'t Text) -> (Layout<Ink>, Shown<'t>, bool, Vec<LinearRgba>) {
        let mut missing_font = false;
        let mut family_of = |fonts: &mut Self, family: &str| {
            if fonts.has_family(family) {
                family.to_owned()
            } else {
                missing_font = true;
                DEFAULT_FAMILY.to_owned()
            }
        };
        let own_style = CharStyle::default();
        let own = text.resolved(&own_style);
        let family = family_of(self, &text.family);
        let mut colors = vec![text.color];
        // Each piece's settings, resolved over the text's.
        let pieces: Vec<(Range<usize>, Resolved<'_>, bool)> = pieces(text)
            .into_iter()
            .map(|(range, style)| {
                (
                    range,
                    text.resolved(style.unwrap_or(&own_style)),
                    style.is_some(),
                )
            })
            .collect();
        let resolved: Vec<(Range<usize>, Resolved<'_>)> =
            pieces.iter().map(|(r, s, _)| (r.clone(), *s)).collect();
        let shown = Shown::new(&text.content, &resolved);
        // The spans' settings, family and ink, in the text laid out.
        let spans: Vec<(Range<usize>, Resolved<'_>, String, Ink)> = pieces
            .iter()
            .filter(|(.., span)| *span)
            .map(|(range, resolved, _)| {
                let family = family_of(self, resolved.family);
                let color = match colors.iter().position(|c| *c == resolved.color) {
                    Some(i) => i,
                    None => {
                        colors.push(resolved.color);
                        colors.len() - 1
                    }
                };
                let color = u16::try_from(color).unwrap_or(u16::MAX);
                let ink = Ink::of(resolved, color);
                let range = shown.shown(range.start)..shown.shown(range.end);
                (range, *resolved, family, ink)
            })
            .collect();
        let mut builder = self
            .layout
            .ranged_builder(&mut self.fonts, &shown.text, 1.0, false);
        for property in properties(&own, &family, 1.0, Ink::of(&own, 0)) {
            builder.push_default(property);
        }
        // Paragraph text breaks a word longer than its box's width, as Photoshop does.
        builder.push_default(StyleProperty::OverflowWrap(OverflowWrap::Anywhere));
        for (range, resolved, family, ink) in &spans {
            for property in properties(resolved, family, 1.0, *ink) {
                builder.push(property, range.clone());
            }
        }
        // Small capitals, smaller within their piece.
        for small in &shown.small {
            let (resolved, family, ink) = spans
                .iter()
                .find(|(range, ..)| range.contains(&small.start))
                .map_or((own, family.as_str(), Ink::of(&own, 0)), |(_, r, f, i)| {
                    (*r, f.as_str(), *i)
                });
            for property in properties(&resolved, family, SMALL_CAPS_SIZE, ink) {
                builder.push(property, small.clone());
            }
        }
        let mut layout: Layout<Ink> = builder.build(&shown.text);
        layout.break_all_lines(text.frame.map(|[width, _]| width as f32));
        let alignment = match text.align {
            TextAlign::Left => Alignment::Left,
            TextAlign::Center => Alignment::Center,
            TextAlign::Right => Alignment::Right,
        };
        layout.align(alignment, AlignmentOptions::default());
        (layout, shown, missing_font, colors)
    }

    /// Where the layout's origin (its top left) lies in the layer's space: point text's anchor
    /// is the first baseline at the alignment point; paragraph text's position, its box's top
    /// left (parley aligns the lines in the box's width).
    fn origin(text: &Text, layout: &Layout<Ink>) -> [f64; 2] {
        if text.frame.is_some() {
            return text.position;
        }
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
        let (layout, _, missing_font, colors) = self.build(text);
        let [ox, oy] = Self::origin(text, &layout);
        // A pen per color.
        let mut pens: Vec<Outlines> = colors.iter().map(|_| Outlines::default()).collect();
        // Whether glyphs' outer contours wind as a rectangle drawn clockwise on screen does,
        // for lines under and through them to add to the glyphs, not cut them (non-zero).
        let mut clockwise = true;
        for line in layout.lines().filter(|line| fits(text, line)) {
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
                let ink = glyphs.style().brush;
                let synthesized = run.synthesis().skew().map_or(0.0, f64::from);
                let slant = if ink.slant { FAUX_ITALIC_DEGREES } else { 0.0 };
                let skew = (synthesized + slant).to_radians().tan();
                let Some(pen) = pens.get_mut(usize::from(ink.color)) else {
                    continue;
                };
                let rise = f64::from(ink.rise);
                let bold = faux_bold_offsets(f64::from(ink.bold));
                let first = pen.subpaths.len();
                for glyph in glyphs.positioned_glyphs() {
                    let Some(outline) = outlines.get(GlyphId::new(glyph.id)) else {
                        continue;
                    };
                    for [dx, dy] in &bold {
                        let x = ox + f64::from(glyph.x) + dx;
                        let y = oy + f64::from(glyph.y) - rise + dy;
                        pen.place = [x, y, skew];
                        let settings = DrawSettings::unhinted(size, LocationRef::new(&coords));
                        // A glyph that cannot be drawn is left out, as a missing glyph would be.
                        let _ = outline.draw(settings, &mut *pen);
                        pen.finish();
                    }
                }
                let area = signed_area(&pen.subpaths[first..]);
                if area != 0.0 {
                    clockwise = area > 0.0;
                }
                // Lines under and through the characters, as long as their advance, as thick
                // and as far from the baseline as the font says: the underline on the line's
                // baseline, the strikethrough through raised or lowered characters.
                let metrics = run.font_metrics();
                let left = ox + f64::from(glyphs.offset());
                let right = left + f64::from(glyphs.advance());
                let baseline = oy + f64::from(glyphs.baseline());
                if ink.underline {
                    let top = baseline - f64::from(metrics.underline_offset);
                    let bottom = top + f64::from(metrics.underline_size);
                    pen.subpaths
                        .push(rectangle(left, top, right, bottom, clockwise));
                }
                if ink.strikethrough {
                    let top = baseline - rise - f64::from(metrics.strikethrough_offset);
                    let bottom = top + f64::from(metrics.strikethrough_size);
                    pen.subpaths
                        .push(rectangle(left, top, right, bottom, clockwise));
                }
            }
        }
        let path = |subpaths: Vec<Subpath>, color: LinearRgba| Shape {
            geometry: Geometry::Path {
                subpaths,
                rule: FillRule::NonZero,
            },
            fill: Some(Paint::Solid(color)),
            stroke: None,
        };
        let several = pens.iter().skip(1).any(|p| !p.subpaths.is_empty());
        let paints = if several {
            pens.iter()
                .zip(&colors)
                .filter(|(p, _)| !p.subpaths.is_empty())
                .map(|(p, &color)| path(p.subpaths.clone(), color))
                .collect()
        } else {
            Vec::new()
        };
        let all = pens.into_iter().flat_map(|p| p.subpaths).collect();
        Laid {
            outlines: path(all, text.color),
            paints,
            missing_font,
        }
    }

    /// Where a caret can stand in `text` and its lines, for editing it (see [`TextGeometry`]).
    pub fn geometry(&mut self, text: &Text) -> TextGeometry {
        let (layout, shown, ..) = self.build(text);
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
            let cursor = Cursor::from_byte_index(&layout, shown.shown(byte), Affinity::Downstream);
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
                    start: utf16(shown.content(range.start)),
                    end: utf16(shown.content(range.end)),
                    hard_break: line.break_reason() == BreakReason::Explicit,
                    left: ox + left,
                    right: ox + left + f64::from(m.advance),
                    top: oy + top,
                    bottom: oy + bottom,
                }
            })
            .collect();
        let overflow = layout.lines().any(|line| !fits(text, &line));
        TextGeometry {
            carets,
            lines,
            overflow,
        }
    }
}

/// Whether `line` shows: paragraph text hides the lines whose letters do not fit in its box's
/// height (the bottom of their descenders), as Photoshop does.
fn fits(text: &Text, line: &parley::Line<'_, Ink>) -> bool {
    text.frame.is_none_or(|[_, height]| {
        f64::from(line.metrics().content_block_max_coord) <= height + 1e-3
    })
}

/// Where Faux Bold draws a glyph again to thicken it by `radius`: in place, and around a circle
/// of that radius (their union is the glyph grown by it, within 8 % of the radius at its
/// corners). Only in place when `radius` is 0.
fn faux_bold_offsets(radius: f64) -> Vec<[f64; 2]> {
    let mut offsets = vec![[0.0, 0.0]];
    if radius > 0.0 {
        offsets.extend((0..8).map(|i| {
            let angle = f64::from(i) * std::f64::consts::FRAC_PI_4;
            [radius * angle.cos(), radius * angle.sin()]
        }));
    }
    offsets
}

/// The area `subpaths` enclose, signed by how they wind (positive clockwise on screen, y
/// down), from their points (control points included: enough for the sign).
fn signed_area(subpaths: &[Subpath]) -> f64 {
    let mut area = 0.0;
    for subpath in subpaths {
        let mut last = subpath.start;
        let mut add = |p: [f64; 2]| {
            area += last[0] * p[1] - p[0] * last[1];
            last = p;
        };
        for segment in &subpath.segments {
            match *segment {
                Segment::Line(p) => add(p),
                Segment::Cubic(a, b, p) => {
                    add(a);
                    add(b);
                    add(p);
                }
            }
        }
        add(subpath.start);
    }
    area / 2.0
}

/// A rectangle from `left` to `right` and `top` to `bottom`, wound clockwise on screen or not.
fn rectangle(left: f64, top: f64, right: f64, bottom: f64, clockwise: bool) -> Subpath {
    let (left, top, right, bottom) = (round(left), round(top), round(right), round(bottom));
    let corners = if clockwise {
        [[right, top], [right, bottom], [left, bottom]]
    } else {
        [[left, bottom], [right, bottom], [right, top]]
    };
    Subpath {
        start: [left, top],
        segments: corners.into_iter().map(Segment::Line).collect(),
        closed: true,
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
