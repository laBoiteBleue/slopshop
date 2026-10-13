//! What a text layer says (ADR 0041): its string and how it is set. Laying it out into glyph
//! outlines needs fonts, which the engine's core does not read: `slopshop-text` does it, and the
//! outlines are kept with the description in a [`super::ShapeSource`], so that every computer
//! draws them the same, the fonts installed or not.

use crate::color::LinearRgba;

/// The longest text a layer holds, in bytes.
pub const MAX_TEXT_LEN: usize = 1 << 20;

/// The longest font family name, in bytes.
pub const MAX_FAMILY_LEN: usize = 256;

/// The smallest and largest type size, in pixels (Photoshop takes 0.01 to 1296 points; at
/// 300 ppi the largest is 5400 pixels).
pub const MIN_SIZE: f64 = 0.01;
pub const MAX_SIZE: f64 = 10_000.0;

/// Photoshop's tracking range, in thousandths of an em.
pub const MAX_TRACKING: f64 = 1000.0;
pub const MIN_TRACKING: f64 = -1000.0;

/// Where lines lie against the text's anchor (Photoshop's paragraph alignment).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    /// Lines start at the anchor.
    Left,
    /// Lines are centered on the anchor.
    Center,
    /// Lines end at the anchor.
    Right,
}

/// A text layer's text: point text, one style for the whole string (per-character styles and
/// box text come later).
#[derive(Debug, Clone, PartialEq)]
pub struct Text {
    /// The string; `\n` breaks lines.
    pub content: String,
    /// The font family's name, as the system or the bundled fonts know it.
    pub family: String,
    /// The font's weight, 1 to 1000 (400 regular, 700 bold).
    pub weight: f32,
    pub italic: bool,
    /// The type size (the em), in pixels. The interface shows points through the document's
    /// resolution (ADR 0028).
    pub size: f64,
    pub color: LinearRgba,
    /// Space added between characters, in thousandths of an em (Photoshop's tracking).
    pub tracking: f64,
    /// The distance between baselines, in pixels; `None`: automatic (120 % of the size, as
    /// Photoshop's Auto).
    pub leading: Option<f64>,
    pub align: TextAlign,
    /// The anchor, in the layer's own space: where the first line's baseline meets the
    /// alignment point.
    pub position: [f64; 2],
}

impl Text {
    pub fn is_valid(&self) -> bool {
        let c = self.color;
        self.content.len() <= MAX_TEXT_LEN
            && !self.family.is_empty()
            && self.family.len() <= MAX_FAMILY_LEN
            && self.weight.is_finite()
            && (1.0..=1000.0).contains(&self.weight)
            && self.size.is_finite()
            && (MIN_SIZE..=MAX_SIZE).contains(&self.size)
            && [c.r, c.g, c.b, c.a].iter().all(|v| v.is_finite())
            && (0.0..=1.0).contains(&c.a)
            && self.tracking.is_finite()
            && (MIN_TRACKING..=MAX_TRACKING).contains(&self.tracking)
            && self
                .leading
                .is_none_or(|l| l.is_finite() && (0.0..=super::model::MAX_COORDINATE).contains(&l))
            && self
                .position
                .iter()
                .all(|v| v.is_finite() && v.abs() <= super::model::MAX_COORDINATE)
    }

    /// The distance between baselines, in pixels.
    pub fn line_spacing(&self) -> f64 {
        self.leading.unwrap_or(self.size * 1.2)
    }
}

/// The longest layer name taken from a text, in characters.
const NAME_LEN: usize = 60;

/// The name Photoshop gives a type layer: its text on one line, shortened.
pub fn layer_name(content: &str) -> String {
    let words: Vec<&str> = content.split_whitespace().collect();
    let line = words.join(" ");
    match line.char_indices().nth(NAME_LEN) {
        Some((end, _)) => line[..end].trim_end().to_owned(),
        None => line,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text() -> Text {
        Text {
            content: "Hello".to_owned(),
            family: "Inter".to_owned(),
            weight: 400.0,
            italic: false,
            size: 48.0,
            color: LinearRgba::new(0.0, 0.0, 0.0, 1.0),
            tracking: 0.0,
            leading: None,
            align: TextAlign::Left,
            position: [10.0, 60.0],
        }
    }

    #[test]
    fn texts_are_validated() {
        assert!(text().is_valid());
        assert!(
            Text {
                content: String::new(),
                ..text()
            }
            .is_valid(),
            "an empty text is valid while it is typed"
        );
        for bad in [
            Text {
                family: String::new(),
                ..text()
            },
            Text {
                size: 0.0,
                ..text()
            },
            Text {
                size: f64::NAN,
                ..text()
            },
            Text {
                weight: 0.0,
                ..text()
            },
            Text {
                tracking: 5000.0,
                ..text()
            },
            Text {
                leading: Some(-1.0),
                ..text()
            },
            Text {
                position: [f64::INFINITY, 0.0],
                ..text()
            },
            Text {
                color: LinearRgba::new(0.0, 0.0, 0.0, 2.0),
                ..text()
            },
        ] {
            assert!(!bad.is_valid(), "{bad:?}");
        }
    }

    #[test]
    fn auto_leading_is_120_percent() {
        assert_eq!(text().line_spacing(), 48.0 * 1.2);
        let set = Text {
            leading: Some(30.0),
            ..text()
        };
        assert_eq!(set.line_spacing(), 30.0);
    }

    #[test]
    fn layers_are_named_from_their_text_on_one_line() {
        assert_eq!(layer_name("Hello\nworld  "), "Hello world");
        assert_eq!(layer_name("  \n "), "");
        let long = "a".repeat(100);
        assert_eq!(layer_name(&long).chars().count(), 60);
        assert_eq!(layer_name("é".repeat(70).as_str()).chars().count(), 60);
    }
}
