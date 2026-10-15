//! Text layers over the IPC (ADR 0041): what the Type tool sends and the options bar shows, the
//! fonts to choose from, and where the caret stands. Laying text out takes the application's
//! fonts, kept once behind a lock (listing the system's fonts takes a moment the first time).

use std::sync::{LazyLock, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use slopshop_core::document::{Document, LayerContent};
use slopshop_core::shape::{CharStyle, Shape, Text, TextAlign, TextSpan};
use slopshop_core::{LayerId, LinearRgba};
use slopshop_text::Fonts;
use tauri::State;

use crate::AppState;

static FONTS: LazyLock<Mutex<Fonts>> = LazyLock::new(|| Mutex::new(Fonts::system()));

/// The application's fonts. Blocking: the first call lists the system's fonts.
pub fn fonts() -> Result<MutexGuard<'static, Fonts>, String> {
    FONTS
        .lock()
        .map_err(|_| "font state is poisoned".to_owned())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextAlignDto {
    Left,
    Center,
    Right,
}

/// A text as the UI sends and reads it: sizes in pixels (the UI shows points through the
/// document's resolution), the color sRGB-encoded RGBA in `[0, 1]` as fill layers', the
/// anchor in the layer's own space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextDto {
    pub content: String,
    pub family: String,
    pub weight: f32,
    pub italic: bool,
    pub size: f64,
    pub color: [f32; 4],
    /// Thousandths of an em.
    #[serde(default)]
    pub tracking: f64,
    /// Pixels; none: automatic.
    #[serde(default)]
    pub leading: Option<f64>,
    pub align: TextAlignDto,
    /// Point text: the anchor; paragraph text: its box's top left corner.
    pub position: [f64; 2],
    /// Paragraph text: its box's width and height; none: point text.
    #[serde(default)]
    pub frame: Option<[f64; 2]>,
    /// Characters set otherwise, in UTF-16 code units of `content`. Views give them; `setText`
    /// keeps the layer's own, following the content's change, and `styleText` changes them.
    #[serde(default)]
    pub spans: Vec<SpanDto>,
}

/// What characters set otherwise than their text: absent fields are the text's. The color is
/// sRGB-encoded RGBA in `[0, 1]`, sizes and leading in pixels (`leading` null: automatic),
/// tracking in thousandths of an em.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharStyleDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[f32; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracking: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "present_or_null"
    )]
    pub leading: Option<Option<f64>>,
}

/// A field that is absent (`None`), null (`Some(None)`) or a value.
mod present_or_null {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(value: &Option<Option<f64>>, s: S) -> Result<S::Ok, S::Error> {
        value.flatten().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<f64>>, D::Error> {
        Option::<f64>::deserialize(d).map(Some)
    }
}

/// Characters `start..end` (UTF-16 code units) set in `style`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpanDto {
    pub start: usize,
    pub end: usize,
    pub style: CharStyleDto,
}

impl CharStyleDto {
    /// The style it describes, validated.
    pub fn style(&self) -> Result<CharStyle, String> {
        let style = CharStyle {
            family: self.family.clone(),
            weight: self.weight,
            italic: self.italic,
            size: self.size,
            color: self
                .color
                .map(|[r, g, b, a]| LinearRgba::from_srgb_encoded_to_working(r, g, b, a)),
            tracking: self.tracking,
            leading: self.leading,
        };
        if style.is_valid() {
            Ok(style)
        } else {
            Err("invalid character style".to_owned())
        }
    }

    pub fn of(style: &CharStyle) -> Self {
        Self {
            family: style.family.clone(),
            weight: style.weight,
            italic: style.italic,
            size: style.size,
            color: style.color.map(|c| c.working_to_srgb_encoded()),
            tracking: style.tracking,
            leading: style.leading,
        }
    }
}

/// The byte offset of UTF-16 place `index` in `content`; `None` past its end or inside a
/// character.
pub fn byte_of(content: &str, index: usize) -> Option<usize> {
    let mut units = 0;
    for (byte, c) in content.char_indices() {
        if units == index {
            return Some(byte);
        }
        units += c.len_utf16();
        if units > index {
            return None;
        }
    }
    (units == index).then_some(content.len())
}

/// The UTF-16 place of byte offset `byte` in `content`.
fn utf16_of(content: &str, byte: usize) -> usize {
    content
        .get(..byte.min(content.len()))
        .map_or(0, |s| s.encode_utf16().count())
}

impl TextDto {
    /// The text it describes, validated.
    pub fn text(&self) -> Result<Text, String> {
        let [r, g, b, a] = self.color;
        let text = Text {
            content: self.content.clone(),
            family: self.family.clone(),
            weight: self.weight,
            italic: self.italic,
            size: self.size,
            color: LinearRgba::from_srgb_encoded_to_working(r, g, b, a),
            tracking: self.tracking,
            leading: self.leading,
            align: match self.align {
                TextAlignDto::Left => TextAlign::Left,
                TextAlignDto::Center => TextAlign::Center,
                TextAlignDto::Right => TextAlign::Right,
            },
            position: self.position,
            frame: self.frame,
            spans: self
                .spans
                .iter()
                .map(|s| {
                    Ok(TextSpan {
                        start: byte_of(&self.content, s.start).ok_or("a span out of the text")?,
                        end: byte_of(&self.content, s.end).ok_or("a span out of the text")?,
                        style: s.style.style()?,
                    })
                })
                .collect::<Result<_, String>>()?,
        };
        if text.is_valid() {
            Ok(text)
        } else {
            Err("invalid text".to_owned())
        }
    }

    /// `text` as the UI sees it.
    pub fn of(text: &Text) -> Self {
        Self {
            content: text.content.clone(),
            family: text.family.clone(),
            weight: text.weight,
            italic: text.italic,
            size: text.size,
            color: text.color.working_to_srgb_encoded(),
            tracking: text.tracking,
            leading: text.leading,
            align: match text.align {
                TextAlign::Left => TextAlignDto::Left,
                TextAlign::Center => TextAlignDto::Center,
                TextAlign::Right => TextAlignDto::Right,
            },
            position: text.position,
            frame: text.frame,
            spans: text
                .spans
                .iter()
                .map(|s| SpanDto {
                    start: utf16_of(&text.content, s.start),
                    end: utf16_of(&text.content, s.end),
                    style: CharStyleDto::of(&s.style),
                })
                .collect(),
        }
    }
}

/// `text` laid out: its outlines, and its glyphs by color when it has several. Blocking (see
/// [`fonts`]).
pub fn lay_out(text: &Text) -> Result<(Shape, Vec<Shape>), String> {
    let laid = fonts()?.lay_out(text);
    Ok((laid.outlines, laid.paints))
}

/// The text of text layer `id` in `document`.
pub fn text_of(document: &Document, id: LayerId) -> Result<&Text, String> {
    let layer = document.layer(id).ok_or("unknown layer")?;
    let LayerContent::Vector { source, .. } = &layer.content else {
        return Err("not a text layer".to_owned());
    };
    source
        .text_of()
        .ok_or_else(|| "not a text layer".to_owned())
}

/// The characters of UTF-16 range `range` of `text` in bytes, or `None` for the whole text;
/// an empty range or one out of the text is refused.
pub fn byte_range(
    text: &Text,
    range: Option<[usize; 2]>,
) -> Result<Option<std::ops::Range<usize>>, String> {
    let Some([start, end]) = range else {
        return Ok(None);
    };
    let start = byte_of(&text.content, start).ok_or("a range out of the text")?;
    let end = byte_of(&text.content, end).ok_or("a range out of the text")?;
    if start >= end {
        return Err("an empty range".to_owned());
    }
    Ok(Some(start..end))
}

/// One style of a family, for the options bar.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct FontStyleView {
    pub weight: f32,
    pub italic: bool,
}

/// The font families to choose from (the system's and the bundled ones), sorted, and the
/// default one.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FontsView {
    pub families: Vec<String>,
    pub default_family: &'static str,
}

#[tauri::command]
pub async fn font_families() -> Result<FontsView, String> {
    tauri::async_runtime::spawn_blocking(|| {
        Ok(FontsView {
            families: fonts()?.families(),
            default_family: slopshop_text::DEFAULT_FAMILY,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The styles `family` offers, upright first, lightest first; empty for an unknown family.
#[tauri::command]
pub async fn font_styles(family: String) -> Result<Vec<FontStyleView>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        Ok(fonts()?
            .styles(&family)
            .into_iter()
            .map(|s| FontStyleView {
                weight: s.weight,
                italic: s.italic,
            })
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Where the caret can stand in a text layer, for the Type tool.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextLayoutView {
    /// One caret per place in the text, counted in UTF-16 code units as the UI's strings are:
    /// `[top x, top y, bottom x, bottom y]` in document pixels.
    pub carets: Vec<[f64; 4]>,
    /// The lines, in order (see [`TextLineView`]).
    pub lines: Vec<TextLineView>,
    /// Paragraph text's box: its corners in document pixels, clockwise from the top left
    /// (the layer's top left, top right, bottom right, bottom left); none for point text.
    pub frame: Option<[[f64; 2]; 4]>,
    /// Paragraph text whose last lines do not fit in its box.
    pub overflow: bool,
    /// The layer's font is not installed: it is drawn as saved, and set in the default family
    /// if it is edited.
    pub missing_font: bool,
}

/// A line of a text, for drawing the selection and moving the caret by lines.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextLineView {
    /// Its first place and the place after its last character (UTF-16 code units); a line
    /// ended by a line break holds it.
    pub start: usize,
    pub end: usize,
    /// It ends at a line break, not where the text wrapped.
    pub hard_break: bool,
    /// Its box's corners in document pixels: top left, top right, bottom right, bottom left
    /// (the start and end of its content, the top and bottom of its line).
    pub corners: [[f64; 2]; 4],
}

/// The carets and lines of text layer `id` (see [`TextLayoutView`]).
pub fn text_layout_of(document: &Document, id: LayerId) -> Result<TextLayoutView, String> {
    let layer = document.layer(id).ok_or("unknown layer")?;
    let LayerContent::Vector { source, .. } = &layer.content else {
        return Err("not a text layer".to_owned());
    };
    let text = source.text_of().ok_or("not a text layer")?;
    // From the layer's space to the document's.
    let to_document = layer.transform.then(document.parent_transform(id));
    let mut fonts = fonts()?;
    let missing_font = !fonts.has_family(&text.family);
    let geometry = fonts.geometry(text);
    let point = |x: f64, y: f64| {
        let (x, y) = to_document.apply(x, y);
        [x, y]
    };
    let carets = geometry
        .carets
        .into_iter()
        .map(|c| {
            let [tx, ty] = point(c.top[0], c.top[1]);
            let [bx, by] = point(c.bottom[0], c.bottom[1]);
            [tx, ty, bx, by]
        })
        .collect();
    let lines = geometry
        .lines
        .into_iter()
        .map(|l| TextLineView {
            start: l.start,
            end: l.end,
            hard_break: l.hard_break,
            corners: [
                point(l.left, l.top),
                point(l.right, l.top),
                point(l.right, l.bottom),
                point(l.left, l.bottom),
            ],
        })
        .collect();
    let frame = text.frame.map(|[w, h]| {
        let [x, y] = text.position;
        [
            point(x, y),
            point(x + w, y),
            point(x + w, y + h),
            point(x, y + h),
        ]
    });
    Ok(TextLayoutView {
        carets,
        lines,
        frame,
        overflow: geometry.overflow,
        missing_font,
    })
}

#[tauri::command]
pub async fn text_layout(
    state: State<'_, AppState>,
    document_id: u64,
    id: u64,
) -> Result<TextLayoutView, String> {
    // A snapshot of the document (its layers are shared, not copied), laid out off the lock.
    let document = {
        let mut documents = state.documents()?;
        documents.get_mut(document_id)?.session.document().clone()
    };
    tauri::async_runtime::spawn_blocking(move || text_layout_of(&document, LayerId::from_raw(id)))
        .await
        .map_err(|e| e.to_string())?
}
