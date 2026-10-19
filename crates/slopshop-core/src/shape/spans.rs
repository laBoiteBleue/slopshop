//! Per-character styles of a text (ADR 0041): spans of characters set differently from the
//! text's own settings, as Photoshop's character attributes. A span sets only what differs
//! (`None`: the text's); spans are kept sorted, apart and merged, in UTF-8 byte offsets of the
//! text's content, on character boundaries.
//!
//! Editing keeps them with the characters: typed characters take the style of the character
//! before them (or of the first one replaced), as in Photoshop; a style applied to a range
//! splits the spans at its ends.

use std::ops::Range;

use super::text::{
    MAX_FAMILY_LEN, MAX_SIZE, MAX_TRACKING, MIN_SIZE, MIN_TRACKING, Text, TextBaseline, TextCase,
};
use crate::color::LinearRgba;

/// What a span of characters sets differently from its text: each field `None` takes the
/// text's.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CharStyle {
    pub family: Option<String>,
    pub weight: Option<f32>,
    pub italic: Option<bool>,
    /// Pixels.
    pub size: Option<f64>,
    pub color: Option<LinearRgba>,
    /// Thousandths of an em.
    pub tracking: Option<f64>,
    /// Pixels between baselines; `Some(None)`: automatic (120 % of the size).
    pub leading: Option<Option<f64>>,
    pub faux_bold: Option<bool>,
    pub faux_italic: Option<bool>,
    pub case: Option<TextCase>,
    pub baseline: Option<TextBaseline>,
    pub underline: Option<bool>,
    pub strikethrough: Option<bool>,
    pub ligatures: Option<bool>,
}

/// Characters `start..end` (UTF-8 bytes of the content) set in `style`.
#[derive(Debug, Clone, PartialEq)]
pub struct TextSpan {
    pub start: usize,
    pub end: usize,
    pub style: CharStyle,
}

/// A character's settings once its span and its text are combined, for laying it out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Resolved<'a> {
    pub family: &'a str,
    pub weight: f32,
    pub italic: bool,
    pub size: f64,
    pub color: LinearRgba,
    pub tracking: f64,
    /// Pixels between baselines (automatic leading resolved).
    pub line_spacing: f64,
    pub faux_bold: bool,
    pub faux_italic: bool,
    pub case: TextCase,
    pub baseline: TextBaseline,
    pub underline: bool,
    pub strikethrough: bool,
    pub ligatures: bool,
}

impl CharStyle {
    /// Whether it sets nothing.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Whether what it sets is in the ranges a text takes.
    pub fn is_valid(&self) -> bool {
        self.family
            .as_ref()
            .is_none_or(|f| !f.is_empty() && f.len() <= MAX_FAMILY_LEN)
            && self
                .weight
                .is_none_or(|w| w.is_finite() && (1.0..=1000.0).contains(&w))
            && self
                .size
                .is_none_or(|s| s.is_finite() && (MIN_SIZE..=MAX_SIZE).contains(&s))
            && self.color.is_none_or(|c| {
                [c.r, c.g, c.b, c.a].iter().all(|v| v.is_finite()) && (0.0..=1.0).contains(&c.a)
            })
            && self
                .tracking
                .is_none_or(|t| t.is_finite() && (MIN_TRACKING..=MAX_TRACKING).contains(&t))
            && self.leading.is_none_or(|l| {
                l.is_none_or(|l| l.is_finite() && (0.0..=super::model::MAX_COORDINATE).contains(&l))
            })
    }

    /// `self` with what `over` sets set on top.
    pub fn overlaid(&self, over: &CharStyle) -> CharStyle {
        CharStyle {
            family: over.family.clone().or_else(|| self.family.clone()),
            weight: over.weight.or(self.weight),
            italic: over.italic.or(self.italic),
            size: over.size.or(self.size),
            color: over.color.or(self.color),
            tracking: over.tracking.or(self.tracking),
            leading: over.leading.or(self.leading),
            faux_bold: over.faux_bold.or(self.faux_bold),
            faux_italic: over.faux_italic.or(self.faux_italic),
            case: over.case.or(self.case),
            baseline: over.baseline.or(self.baseline),
            underline: over.underline.or(self.underline),
            strikethrough: over.strikethrough.or(self.strikethrough),
            ligatures: over.ligatures.or(self.ligatures),
        }
    }

    /// `self` without what `other` sets.
    fn without(&self, other: &CharStyle) -> CharStyle {
        CharStyle {
            family: self.family.clone().filter(|_| other.family.is_none()),
            weight: self.weight.filter(|_| other.weight.is_none()),
            italic: self.italic.filter(|_| other.italic.is_none()),
            size: self.size.filter(|_| other.size.is_none()),
            color: self.color.filter(|_| other.color.is_none()),
            tracking: self.tracking.filter(|_| other.tracking.is_none()),
            leading: self.leading.filter(|_| other.leading.is_none()),
            faux_bold: self.faux_bold.filter(|_| other.faux_bold.is_none()),
            faux_italic: self.faux_italic.filter(|_| other.faux_italic.is_none()),
            case: self.case.filter(|_| other.case.is_none()),
            baseline: self.baseline.filter(|_| other.baseline.is_none()),
            underline: self.underline.filter(|_| other.underline.is_none()),
            strikethrough: self.strikethrough.filter(|_| other.strikethrough.is_none()),
            ligatures: self.ligatures.filter(|_| other.ligatures.is_none()),
        }
    }

    /// `self` without what `text` sets the same way: a span sets only what differs.
    fn beyond(&self, text: &Text) -> CharStyle {
        CharStyle {
            family: self.family.clone().filter(|f| *f != text.family),
            weight: self.weight.filter(|w| *w != text.weight),
            italic: self.italic.filter(|i| *i != text.italic),
            size: self.size.filter(|s| *s != text.size),
            color: self.color.filter(|c| *c != text.color),
            tracking: self.tracking.filter(|t| *t != text.tracking),
            leading: self.leading.filter(|l| *l != text.leading),
            faux_bold: self.faux_bold.filter(|b| *b != text.faux_bold),
            faux_italic: self.faux_italic.filter(|i| *i != text.faux_italic),
            case: self.case.filter(|c| *c != text.case),
            baseline: self.baseline.filter(|b| *b != text.baseline),
            underline: self.underline.filter(|u| *u != text.underline),
            strikethrough: self.strikethrough.filter(|s| *s != text.strikethrough),
            ligatures: self.ligatures.filter(|l| *l != text.ligatures),
        }
    }
}

/// A run of characters and its style, the text's own (empty) where no span is.
type Run = (usize, usize, CharStyle);

impl Text {
    /// The settings of characters set in `style` (a span's, or none for the text's own).
    pub fn resolved<'a>(&'a self, style: &'a CharStyle) -> Resolved<'a> {
        let size = style.size.unwrap_or(self.size);
        Resolved {
            family: style.family.as_deref().unwrap_or(&self.family),
            weight: style.weight.unwrap_or(self.weight),
            italic: style.italic.unwrap_or(self.italic),
            size,
            color: style.color.unwrap_or(self.color),
            tracking: style.tracking.unwrap_or(self.tracking),
            line_spacing: style.leading.unwrap_or(self.leading).unwrap_or(size * 1.2),
            faux_bold: style.faux_bold.unwrap_or(self.faux_bold),
            faux_italic: style.faux_italic.unwrap_or(self.faux_italic),
            case: style.case.unwrap_or(self.case),
            baseline: style.baseline.unwrap_or(self.baseline),
            underline: style.underline.unwrap_or(self.underline),
            strikethrough: style.strikethrough.unwrap_or(self.strikethrough),
            ligatures: style.ligatures.unwrap_or(self.ligatures),
        }
    }

    /// Whether the spans are sorted, apart, within the content on character boundaries, and
    /// each sets something valid.
    pub(super) fn spans_are_valid(&self) -> bool {
        let mut last = 0;
        self.spans.iter().all(|s| {
            let ok = s.start >= last
                && s.start < s.end
                && s.end <= self.content.len()
                && self.content.is_char_boundary(s.start)
                && self.content.is_char_boundary(s.end)
                && !s.style.is_empty()
                && s.style.is_valid();
            last = s.end;
            ok
        })
    }

    /// The whole content in runs, gaps between spans as runs of the text's own style.
    fn runs(&self) -> Vec<Run> {
        let mut runs = Vec::with_capacity(self.spans.len() * 2 + 1);
        let mut at = 0;
        for s in &self.spans {
            if s.start > at {
                runs.push((at, s.start, CharStyle::default()));
            }
            runs.push((s.start, s.end, s.style.clone()));
            at = s.end;
        }
        if at < self.content.len() {
            runs.push((at, self.content.len(), CharStyle::default()));
        }
        runs
    }

    /// The spans of `runs`: only what differs from the text, neighbors set alike merged.
    fn set_runs(&mut self, runs: Vec<Run>) {
        let mut spans: Vec<TextSpan> = Vec::new();
        for (start, end, style) in runs {
            let style = style.beyond(self);
            if start >= end || style.is_empty() {
                continue;
            }
            match spans.last_mut() {
                Some(last) if last.end == start && last.style == style => last.end = end,
                _ => spans.push(TextSpan { start, end, style }),
            }
        }
        self.spans = spans;
    }

    /// The style of the character at byte `index` (its span's, or the text's own).
    fn style_at(&self, index: usize) -> CharStyle {
        self.spans
            .iter()
            .find(|s| s.start <= index && index < s.end)
            .map(|s| s.style.clone())
            .unwrap_or_default()
    }

    /// The text saying `content`, its spans following the change: the characters kept keep
    /// their styles; the characters typed take the style of the first one replaced, else of
    /// the one before them (the first one's at the start), as in Photoshop.
    pub fn with_content(&self, content: String) -> Text {
        let old = &self.content;
        // The change: old[a..b] became content[a..a + inserted], found by the common ends.
        let mut a = old
            .bytes()
            .zip(content.bytes())
            .take_while(|(x, y)| x == y)
            .count();
        while !(old.is_char_boundary(a) && content.is_char_boundary(a)) {
            a -= 1;
        }
        let max_suffix = old.len().min(content.len()) - a;
        let mut suffix = old
            .bytes()
            .rev()
            .zip(content.bytes().rev())
            .take(max_suffix)
            .take_while(|(x, y)| x == y)
            .count();
        while !(old.is_char_boundary(old.len() - suffix)
            && content.is_char_boundary(content.len() - suffix))
        {
            suffix -= 1;
        }
        let b = old.len() - suffix;
        let inserted = content.len() - suffix - a;
        let typed_style = if b > a {
            self.style_at(a)
        } else if a > 0 {
            let before = old[..a].char_indices().next_back().map_or(0, |(i, _)| i);
            self.style_at(before)
        } else {
            self.style_at(0)
        };
        let mut runs = Vec::new();
        for (s, e, style) in self.runs() {
            if s < a {
                runs.push((s, e.min(a), style.clone()));
            }
            if e > b {
                let (s, e) = (s.max(b), e);
                runs.push((s - b + a + inserted, e - b + a + inserted, style));
            }
        }
        if inserted > 0 {
            runs.push((a, a + inserted, typed_style));
        }
        runs.sort_by_key(|r| r.0);
        let mut text = Text {
            content,
            spans: Vec::new(),
            ..self.clone()
        };
        text.set_runs(runs);
        text
    }

    /// `wanted`'s content and settings with this text's spans, following the content's change
    /// (see [`Self::with_content`]); what `wanted`'s spans say is not taken. What typing, the
    /// box's handles and the paragraph settings send.
    pub fn retyped(&self, wanted: Text) -> Text {
        let runs = self.with_content(wanted.content.clone()).runs();
        let mut text = Text {
            spans: Vec::new(),
            ..wanted
        };
        text.set_runs(runs);
        text
    }

    /// The text with `change` applied to the characters of `range` (UTF-8 bytes of the content,
    /// on character boundaries), or with `None` to the whole text: its own settings change, and
    /// the spans stop setting what `change` sets.
    pub fn styled(&self, range: Option<Range<usize>>, change: &CharStyle) -> Text {
        let mut text = self.clone();
        let Some(range) = range else {
            let style = CharStyle::default().overlaid(change);
            if let Some(family) = style.family {
                text.family = family;
            }
            text.weight = style.weight.unwrap_or(text.weight);
            text.italic = style.italic.unwrap_or(text.italic);
            text.size = style.size.unwrap_or(text.size);
            text.color = style.color.unwrap_or(text.color);
            text.tracking = style.tracking.unwrap_or(text.tracking);
            text.leading = style.leading.unwrap_or(text.leading);
            text.faux_bold = style.faux_bold.unwrap_or(text.faux_bold);
            text.faux_italic = style.faux_italic.unwrap_or(text.faux_italic);
            text.case = style.case.unwrap_or(text.case);
            text.baseline = style.baseline.unwrap_or(text.baseline);
            text.underline = style.underline.unwrap_or(text.underline);
            text.strikethrough = style.strikethrough.unwrap_or(text.strikethrough);
            text.ligatures = style.ligatures.unwrap_or(text.ligatures);
            let runs = self
                .runs()
                .into_iter()
                .map(|(s, e, style)| (s, e, style.without(change)))
                .collect();
            text.set_runs(runs);
            return text;
        };
        let mut runs = Vec::new();
        for (s, e, style) in self.runs() {
            // The run's parts before, in and after the range.
            for (from, to, inside) in [
                (s, e.min(range.start), false),
                (s.max(range.start), e.min(range.end), true),
                (s.max(range.end), e, false),
            ] {
                if from < to {
                    let style = if inside {
                        style.overlaid(change)
                    } else {
                        style.clone()
                    };
                    runs.push((from, to, style));
                }
            }
        }
        text.set_runs(runs);
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shape::TextAlign;

    fn text(content: &str) -> Text {
        Text {
            content: content.to_owned(),
            family: "Inter".to_owned(),
            weight: 400.0,
            italic: false,
            size: 48.0,
            color: LinearRgba::new(0.0, 0.0, 0.0, 1.0),
            tracking: 0.0,
            leading: None,
            faux_bold: false,
            faux_italic: false,
            case: crate::shape::TextCase::Normal,
            baseline: crate::shape::TextBaseline::Normal,
            underline: false,
            strikethrough: false,
            ligatures: true,
            align: TextAlign::Left,
            position: [10.0, 60.0],
            frame: None,
            spans: Vec::new(),
        }
    }

    fn bold() -> CharStyle {
        CharStyle {
            weight: Some(700.0),
            ..CharStyle::default()
        }
    }

    fn red() -> CharStyle {
        CharStyle {
            color: Some(LinearRgba::new(1.0, 0.0, 0.0, 1.0)),
            ..CharStyle::default()
        }
    }

    fn span(start: usize, end: usize, style: CharStyle) -> TextSpan {
        TextSpan { start, end, style }
    }

    #[test]
    fn a_style_on_a_range_makes_a_span_and_splits_the_others() {
        let hello = text("Hello world");
        let styled = hello.styled(Some(0..5), &bold());
        assert_eq!(styled.spans, [span(0, 5, bold())]);
        assert!(styled.is_valid());
        // Red over "lo wo": the bold span splits, the overlap is both.
        let both = styled.styled(Some(3..8), &red());
        assert_eq!(
            both.spans,
            [
                span(0, 3, bold()),
                span(3, 5, bold().overlaid(&red())),
                span(5, 8, red()),
            ]
        );
        // Back to the text's own weight: the span sets only the color.
        let regular = CharStyle {
            weight: Some(400.0),
            ..CharStyle::default()
        };
        assert_eq!(both.styled(Some(0..5), &regular).spans, [span(3, 8, red())]);
    }

    #[test]
    fn a_style_on_the_whole_text_changes_it_and_clears_the_spans_of_it() {
        let styled = text("Hello").styled(Some(0..2), &bold().overlaid(&red()));
        let whole = styled.styled(None, &bold());
        assert_eq!(whole.weight, 700.0);
        assert_eq!(whole.spans, [span(0, 2, red())]);
        let auto = CharStyle {
            leading: Some(None),
            ..CharStyle::default()
        };
        let set = text("Hi").styled(
            None,
            &CharStyle {
                leading: Some(Some(30.0)),
                ..CharStyle::default()
            },
        );
        assert_eq!(set.leading, Some(30.0));
        assert_eq!(set.styled(None, &auto).leading, None);
    }

    #[test]
    fn typed_characters_take_the_style_before_them() {
        let styled = text("Hello world").styled(Some(0..5), &bold());
        // At the end of the bold word: bold.
        assert_eq!(
            styled.with_content("Hello! world".to_owned()).spans,
            [span(0, 6, bold())]
        );
        // After the space: regular, the span unmoved.
        assert_eq!(
            styled.with_content("Hello big world".to_owned()).spans,
            [span(0, 5, bold())]
        );
        // Before the bold word, at the start: the first character's style.
        assert_eq!(
            styled.with_content(">Hello world".to_owned()).spans,
            [span(0, 6, bold())]
        );
        // Characters removed in the span: it shrinks; after it: it stays.
        assert_eq!(
            styled.with_content("Heo world".to_owned()).spans,
            [span(0, 3, bold())]
        );
        assert_eq!(
            styled.with_content("Hello".to_owned()).spans,
            [span(0, 5, bold())]
        );
        // The whole span removed: no span.
        assert!(styled.with_content(" world".to_owned()).spans.is_empty());
    }

    #[test]
    fn a_replaced_selection_takes_the_first_replaced_style() {
        let styled = text("Hello world").styled(Some(6..11), &red());
        // "world" replaced by "you": red.
        assert_eq!(
            styled.with_content("Hello you".to_owned()).spans,
            [span(6, 9, red())]
        );
        // " w" replaced (regular first): regular.
        assert_eq!(
            styled.with_content("Hello_orld".to_owned()).spans,
            [span(6, 10, red())]
        );
    }

    #[test]
    fn spans_follow_multibyte_characters() {
        let styled = text("été ça").styled(Some(0..5), &bold());
        assert!(styled.is_valid());
        let typed = styled.with_content("étés ça".to_owned());
        assert_eq!(typed.spans, [span(0, 6, bold())]);
        assert!(typed.is_valid());
        // A change inside a multibyte character's bytes is a change of the whole character.
        let swapped = text("aé")
            .styled(Some(1..3), &bold())
            .with_content("aè".to_owned());
        assert_eq!(swapped.spans, [span(1, 3, bold())]);
    }

    #[test]
    fn retyping_keeps_the_spans_and_takes_the_new_settings() {
        let styled = text("Hello").styled(Some(0..2), &bold());
        let wanted = Text {
            weight: 700.0,
            size: 20.0,
            spans: vec![span(0, 1, red())],
            ..text("Hello!")
        };
        let retyped = styled.retyped(wanted);
        assert_eq!((retyped.content.as_str(), retyped.size), ("Hello!", 20.0));
        // The span's weight is now the text's: nothing left to set; red not taken.
        assert!(retyped.spans.is_empty(), "{:?}", retyped.spans);
        let kept = styled.retyped(text("Hey"));
        assert_eq!(kept.spans, [span(0, 2, bold())]);
    }

    #[test]
    fn character_styles_set_faux_styles_cases_baselines_lines_and_ligatures() {
        let under = CharStyle {
            underline: Some(true),
            case: Some(TextCase::SmallCaps),
            ..CharStyle::default()
        };
        let styled = text("Hello world").styled(Some(0..5), &under);
        assert_eq!(styled.spans, [span(0, 5, under.clone())]);
        let r = styled.resolved(&styled.spans[0].style);
        assert!(r.underline && r.ligatures && !r.faux_bold);
        assert_eq!(
            (r.case, r.baseline),
            (TextCase::SmallCaps, TextBaseline::Normal)
        );
        // The whole text underlined: the span keeps only its case.
        let whole = styled.styled(
            None,
            &CharStyle {
                underline: Some(true),
                ligatures: Some(false),
                ..CharStyle::default()
            },
        );
        assert!(whole.underline && !whole.ligatures);
        assert_eq!(
            whole.spans,
            [span(
                0,
                5,
                CharStyle {
                    case: Some(TextCase::SmallCaps),
                    ..CharStyle::default()
                }
            )]
        );
        // Back to the text's own case: no span left.
        let normal = CharStyle {
            case: Some(TextCase::Normal),
            ..CharStyle::default()
        };
        assert!(whole.styled(Some(0..5), &normal).spans.is_empty());
        // Typed characters take them too.
        let sup = text("x2").styled(
            Some(1..2),
            &CharStyle {
                baseline: Some(TextBaseline::Superscript),
                faux_bold: Some(true),
                ..CharStyle::default()
            },
        );
        let typed = sup.with_content("x23".to_owned());
        assert_eq!((typed.spans[0].start, typed.spans[0].end), (1, 3));
        assert_eq!(
            typed.spans[0].style.baseline,
            Some(TextBaseline::Superscript)
        );
    }

    #[test]
    fn neighbors_set_alike_merge() {
        let styled = text("abcd")
            .styled(Some(0..2), &bold())
            .styled(Some(2..4), &bold());
        assert_eq!(styled.spans, [span(0, 4, bold())]);
    }

    #[test]
    fn spans_are_validated() {
        let t = text("Hello");
        for bad in [
            vec![span(2, 2, bold())],
            vec![span(0, 9, bold())],
            vec![span(0, 3, bold()), span(2, 4, red())],
            vec![span(3, 4, bold()), span(0, 2, red())],
            vec![span(0, 2, CharStyle::default())],
            vec![span(
                0,
                2,
                CharStyle {
                    size: Some(0.0),
                    ..CharStyle::default()
                },
            )],
        ] {
            let bad = Text {
                spans: bad,
                ..t.clone()
            };
            assert!(!bad.is_valid(), "{:?}", bad.spans);
        }
        let inside = Text {
            spans: vec![span(1, 2, bold())],
            ..text("é")
        };
        assert!(!inside.is_valid(), "not on a character boundary");
    }

    #[test]
    fn characters_resolve_their_span_over_their_text() {
        let t = text("Hi");
        let none = CharStyle::default();
        let own = t.resolved(&none);
        assert_eq!((own.family, own.weight, own.size), ("Inter", 400.0, 48.0));
        assert!((own.line_spacing - 57.6).abs() < 1e-9);
        let big = CharStyle {
            size: Some(100.0),
            family: Some("Arial".to_owned()),
            ..CharStyle::default()
        };
        let r = t.resolved(&big);
        assert_eq!((r.family, r.size), ("Arial", 100.0));
        assert!(
            (r.line_spacing - 120.0).abs() < 1e-9,
            "automatic leading follows the size"
        );
    }
}
