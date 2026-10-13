//! Text layers over the IPC (ADR 0041): what the Type tool sends and the options bar shows, the
//! fonts to choose from, and where the caret stands. Laying text out takes the application's
//! fonts, kept once behind a lock (listing the system's fonts takes a moment the first time).

use std::sync::{LazyLock, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use slopshop_core::document::{Document, LayerContent};
use slopshop_core::shape::{Shape, Text, TextAlign};
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
    pub position: [f64; 2],
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
        }
    }
}

/// `text` laid out: its outlines. Blocking (see [`fonts`]).
pub fn lay_out(text: &Text) -> Result<Shape, String> {
    Ok(fonts()?.lay_out(text).outlines)
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
    /// The layer's font is not installed: it is drawn as saved, and set in the default family
    /// if it is edited.
    pub missing_font: bool,
}

/// The carets of text layer `id` (see [`TextLayoutView`]).
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
    let carets = fonts
        .carets(text)
        .into_iter()
        .map(|c| {
            let (tx, ty) = to_document.apply(c.top[0], c.top[1]);
            let (bx, by) = to_document.apply(c.bottom[0], c.bottom[1]);
            [tx, ty, bx, by]
        })
        .collect();
    Ok(TextLayoutView {
        carets,
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
