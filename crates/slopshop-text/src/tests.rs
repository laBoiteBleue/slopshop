use slopshop_core::color::LinearRgba;
use slopshop_core::shape::CharStyle;
use slopshop_core::shape::path::shape_bounds;

use super::*;

fn text(content: &str) -> Text {
    Text {
        content: content.to_owned(),
        family: DEFAULT_FAMILY.to_owned(),
        weight: 400.0,
        italic: false,
        size: 50.0,
        color: LinearRgba::new(1.0, 0.0, 0.0, 1.0),
        tracking: 0.0,
        leading: None,
        align: TextAlign::Left,
        position: [100.0, 200.0],
        frame: None,
        spans: Vec::new(),
    }
}

fn bounds(laid: &Laid) -> [f64; 4] {
    shape_bounds(&laid.outlines).expect("the text has outlines")
}

#[test]
fn the_bundled_family_is_listed_with_its_styles() {
    let mut fonts = Fonts::bundled();
    assert_eq!(fonts.families(), vec![DEFAULT_FAMILY.to_owned()]);
    let styles = fonts.styles(DEFAULT_FAMILY);
    // Thin to Black, upright then italic.
    assert_eq!(styles.len(), 18, "{styles:?}");
    assert_eq!(
        styles[0],
        FontStyleInfo {
            weight: 100.0,
            italic: false
        }
    );
    assert!(styles[17].italic && styles[17].weight == 900.0);
    assert!(fonts.styles("No Such Font").is_empty());
}

#[test]
fn text_is_laid_out_on_its_baseline_from_its_anchor() {
    let mut fonts = Fonts::bundled();
    let laid = fonts.lay_out(&text("Hello"));
    assert!(!laid.missing_font);
    assert!(laid.outlines.is_valid());
    assert_eq!(
        laid.outlines.fill,
        Some(Paint::Solid(LinearRgba::new(1.0, 0.0, 0.0, 1.0)))
    );
    let [l, t, r, b] = bounds(&laid);
    // "H" starts at the anchor (with its side bearing), capitals rise about 0.73 em above the
    // baseline, and only the overshoot of "o" goes below it.
    assert!((100.0..106.0).contains(&l), "{l}");
    assert!((200.0 - 40.0..200.0 - 33.0).contains(&t), "{t}");
    assert!((199.0..=201.0).contains(&b), "{b}");
    assert!(r - l > 100.0 && r - l < 140.0, "{}", r - l);
}

#[test]
fn alignment_places_lines_against_the_anchor() {
    let mut fonts = Fonts::bundled();
    let left = bounds(&fonts.lay_out(&text("Title")));
    let center = bounds(&fonts.lay_out(&Text {
        align: TextAlign::Center,
        ..text("Title")
    }));
    let right = bounds(&fonts.lay_out(&Text {
        align: TextAlign::Right,
        ..text("Title")
    }));
    let width = left[2] - left[0];
    assert!(
        ((center[0] + center[2]) / 2.0 - 100.0).abs() < 3.0,
        "{center:?}"
    );
    assert!((right[2] - 100.0).abs() < 3.0, "{right:?}");
    assert!(((right[2] - right[0]) - width).abs() < 1e-6);
}

#[test]
fn new_lines_go_down_by_the_leading() {
    let mut fonts = Fonts::bundled();
    let one = bounds(&fonts.lay_out(&text("H")));
    let two = bounds(&fonts.lay_out(&text("H\nH")));
    assert!((two[3] - one[3] - 60.0).abs() < 0.01, "auto: 120 % of 50");
    let set = bounds(&fonts.lay_out(&Text {
        leading: Some(80.0),
        ..text("H\nH")
    }));
    assert!((set[3] - one[3] - 80.0).abs() < 0.01);
}

#[test]
fn tracking_spreads_characters() {
    let mut fonts = Fonts::bundled();
    let plain = bounds(&fonts.lay_out(&text("IIII")));
    let tracked = bounds(&fonts.lay_out(&Text {
        tracking: 200.0,
        ..text("IIII")
    }));
    // Three gaps of 200/1000 em, 10 px each.
    let grown = (tracked[2] - tracked[0]) - (plain[2] - plain[0]);
    assert!((grown - 30.0).abs() < 0.5, "{grown}");
}

#[test]
fn bold_is_wider_and_italic_leans() {
    let mut fonts = Fonts::bundled();
    let regular = bounds(&fonts.lay_out(&text("mmmm")));
    let bold = bounds(&fonts.lay_out(&Text {
        weight: 800.0,
        ..text("mmmm")
    }));
    assert!(bold[2] - bold[0] > regular[2] - regular[0] + 5.0);
    let upright = fonts.lay_out(&text("l"));
    let italic = fonts.lay_out(&Text {
        italic: true,
        ..text("l")
    });
    assert_ne!(upright.outlines, italic.outlines);
}

#[test]
fn a_missing_family_falls_back_to_the_default_one() {
    let mut fonts = Fonts::bundled();
    let laid = fonts.lay_out(&Text {
        family: "No Such Font".to_owned(),
        ..text("Hello")
    });
    assert!(laid.missing_font);
    assert_eq!(laid.outlines, fonts.lay_out(&text("Hello")).outlines);
}

#[test]
fn empty_text_has_no_outlines_but_a_caret() {
    let mut fonts = Fonts::bundled();
    let laid = fonts.lay_out(&text(""));
    assert!(shape_bounds(&laid.outlines).is_none());
    let geometry = fonts.geometry(&text(""));
    let carets = geometry.carets;
    assert_eq!(carets.len(), 1);
    assert_eq!(
        geometry.lines.len(),
        1,
        "an empty text has a line to type in"
    );
    let c = carets[0];
    assert!((c.top[0] - 100.0).abs() < 1e-6);
    assert!(c.top[1] < 200.0 && c.bottom[1] > 200.0, "{c:?}");
}

#[test]
fn carets_follow_the_text_in_utf16_units() {
    let mut fonts = Fonts::bundled();
    let content = "ab\ncd😀";
    let carets = fonts.geometry(&text(content)).carets;
    assert_eq!(carets.len(), content.encode_utf16().count() + 1);
    // Along the first line, left to right.
    assert!(carets[0].top[0] < carets[1].top[0] && carets[1].top[0] < carets[2].top[0]);
    // After the line break: the second line, back at the anchor, a line lower.
    assert!((carets[3].top[0] - 100.0).abs() < 1e-6);
    assert!((carets[3].top[1] - carets[0].top[1] - 60.0).abs() < 0.01);
    // The emoji's two UTF-16 units share the caret before it.
    assert_eq!(carets[5], carets[6]);
}

#[test]
fn lines_give_their_places_and_boxes() {
    let mut fonts = Fonts::bundled();
    let content = "ab\ncd😀";
    let geometry = fonts.geometry(&text(content));
    let lines = &geometry.lines;
    assert_eq!(lines.len(), 2, "{lines:?}");
    // The first line holds its line break; the second ends with the text (UTF-16 units).
    assert_eq!((lines[0].start, lines[0].end), (0, 3));
    assert!(lines[0].hard_break);
    assert_eq!((lines[1].start, lines[1].end), (3, 7));
    assert!(!lines[1].hard_break);
    // Boxes: from the anchor, around the baseline, one line spacing apart, as the carets.
    for (line, caret) in lines.iter().zip([geometry.carets[0], geometry.carets[3]]) {
        assert!((line.left - 100.0).abs() < 1e-6, "{line:?}");
        assert!(line.right > line.left + 20.0, "{line:?}");
        assert!(line.top <= caret.top[1] + 1e-6 && line.bottom >= caret.bottom[1] - 1e-6);
    }
    assert!((lines[1].top - lines[0].top - 60.0).abs() < 0.01);
    // The last caret of the first line stands at its end, before the break.
    assert!((geometry.carets[2].top[0] - lines[0].right).abs() < 1.0);
}

/// Paragraph text in a box at (100, 200), `width` by `height`.
fn boxed(content: &str, width: f64, height: f64) -> Text {
    Text {
        frame: Some([width, height]),
        ..text(content)
    }
}

#[test]
fn paragraph_text_wraps_in_its_box_from_its_top_left() {
    let mut fonts = Fonts::bundled();
    let content = "one two three four five six";
    let point = fonts.geometry(&text(content));
    assert_eq!(point.lines.len(), 1);
    let geometry = fonts.geometry(&boxed(content, 200.0, 1000.0));
    assert!(geometry.lines.len() >= 3, "{:?}", geometry.lines);
    assert!(!geometry.overflow);
    // The lines start at the box's left edge, the first one from its top, and none is wider
    // than the box but for the space it wrapped at (which hangs past it).
    // (Inter's letters reach a little above a 120 % line box.)
    let top = geometry.lines[0].top;
    assert!((195.0..=200.0).contains(&top), "{top}");
    for line in &geometry.lines {
        assert!((line.left - 100.0).abs() < 1e-6, "{line:?}");
        assert!(line.right <= 320.0, "{line:?}");
    }
    let [l, t, r, _] = bounds(&fonts.lay_out(&boxed(content, 200.0, 1000.0)));
    assert!(l >= 100.0 && r <= 300.0 && t >= 195.0, "{l} {t} {r}");
}

#[test]
fn paragraph_text_is_aligned_in_its_box() {
    let mut fonts = Fonts::bundled();
    let centered = Text {
        align: TextAlign::Center,
        ..boxed("Hi", 400.0, 100.0)
    };
    let [l, _, r, _] = bounds(&fonts.lay_out(&centered));
    assert!(((l + r) / 2.0 - 300.0).abs() < 3.0, "{l} {r}");
    let right = Text {
        align: TextAlign::Right,
        ..boxed("Hi", 400.0, 100.0)
    };
    let [_, _, r, _] = bounds(&fonts.lay_out(&right));
    assert!((490.0..=500.0).contains(&r), "{r}");
}

#[test]
fn lines_past_the_box_are_hidden() {
    let mut fonts = Fonts::bundled();
    // Lines 60 px apart (50 px type): two fit in 130 px, not the third.
    let full = boxed("a\nb\nc", 300.0, 1000.0);
    let cut = boxed("a\nb\nc", 300.0, 130.0);
    let geometry = fonts.geometry(&cut);
    assert!(geometry.overflow);
    assert_eq!(geometry.lines.len(), 3, "hidden lines still take the caret");
    let [_, _, _, bottom] = bounds(&fonts.lay_out(&cut));
    assert!(bottom <= 330.0, "{bottom}");
    assert!(bounds(&fonts.lay_out(&full))[3] > 330.0);
    assert!(!fonts.geometry(&full).overflow);
    assert!(
        !fonts.geometry(&text("a\nb\nc")).overflow,
        "point text never overflows"
    );
}

#[test]
fn a_word_longer_than_the_box_is_broken() {
    let mut fonts = Fonts::bundled();
    let geometry = fonts.geometry(&boxed("Supercalifragilistic", 120.0, 1000.0));
    assert!(geometry.lines.len() >= 2, "{:?}", geometry.lines);
    for line in &geometry.lines {
        assert!(line.right <= 220.0 + 1e-3, "{line:?}");
    }
}

#[test]
fn spans_set_their_characters_otherwise() {
    let mut fonts = Fonts::bundled();
    let plain = text("mmmm");
    let bold = plain.styled(
        Some(0..2),
        &CharStyle {
            weight: Some(800.0),
            ..CharStyle::default()
        },
    );
    let [l, _, r, _] = bounds(&fonts.lay_out(&plain));
    let [bl, _, br, _] = bounds(&fonts.lay_out(&bold));
    assert!(br - bl > r - l + 2.0, "{} {}", br - bl, r - l);
    // A span twice as large: its line taller, the text wider.
    let big = plain.styled(
        Some(3..4),
        &CharStyle {
            size: Some(100.0),
            ..CharStyle::default()
        },
    );
    let geometry = fonts.geometry(&big);
    let line = geometry.lines[0];
    assert!(line.bottom - line.top > 100.0, "{line:?}");
    assert!(bounds(&fonts.lay_out(&big))[2] > r + 20.0);
}

#[test]
fn spans_in_other_colors_give_the_glyphs_by_color() {
    let mut fonts = Fonts::bundled();
    let one = fonts.lay_out(&text("ab"));
    assert!(one.paints.is_empty(), "one color: the outlines are drawn");
    let blue = LinearRgba::new(0.0, 0.0, 1.0, 1.0);
    let two = text("ab").styled(
        Some(1..2),
        &CharStyle {
            color: Some(blue),
            ..CharStyle::default()
        },
    );
    let laid = fonts.lay_out(&two);
    assert_eq!(laid.paints.len(), 2);
    assert_eq!(
        laid.paints[0].fill,
        Some(Paint::Solid(LinearRgba::new(1.0, 0.0, 0.0, 1.0)))
    );
    assert_eq!(laid.paints[1].fill, Some(Paint::Solid(blue)));
    // "b" right of "a"; all the glyphs in the outlines, as one color.
    let [_, _, a_right, _] = shape_bounds(&laid.paints[0]).unwrap();
    let [b_left, _, _, _] = shape_bounds(&laid.paints[1]).unwrap();
    assert!(b_left >= a_right - 1.0, "{a_right} {b_left}");
    assert_eq!(laid.outlines, one.outlines);
}

#[test]
fn a_span_in_a_missing_family_is_set_in_the_default_one() {
    let mut fonts = Fonts::bundled();
    let gone = text("ab").styled(
        Some(0..1),
        &CharStyle {
            family: Some("No Such Font".to_owned()),
            ..CharStyle::default()
        },
    );
    let laid = fonts.lay_out(&gone);
    assert!(laid.missing_font);
    assert_eq!(laid.outlines, fonts.lay_out(&text("ab")).outlines);
}
