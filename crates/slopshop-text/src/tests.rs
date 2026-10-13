use slopshop_core::color::LinearRgba;
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
    let carets = fonts.carets(&text(""));
    assert_eq!(carets.len(), 1);
    let c = carets[0];
    assert!((c.top[0] - 100.0).abs() < 1e-6);
    assert!(c.top[1] < 200.0 && c.bottom[1] > 200.0, "{c:?}");
}

#[test]
fn carets_follow_the_text_in_utf16_units() {
    let mut fonts = Fonts::bundled();
    let content = "ab\ncd😀";
    let carets = fonts.carets(&text(content));
    assert_eq!(carets.len(), content.encode_utf16().count() + 1);
    // Along the first line, left to right.
    assert!(carets[0].top[0] < carets[1].top[0] && carets[1].top[0] < carets[2].top[0]);
    // After the line break: the second line, back at the anchor, a line lower.
    assert!((carets[3].top[0] - 100.0).abs() < 1e-6);
    assert!((carets[3].top[1] - carets[0].top[1] - 60.0).abs() < 0.01);
    // The emoji's two UTF-16 units share the caret before it.
    assert_eq!(carets[5], carets[6]);
}
