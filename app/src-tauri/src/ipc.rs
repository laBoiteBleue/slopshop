//! IPC data types: the contract between the UI and the engine.
//!
//! These DTOs are deliberately separate from the core model so that the engine can evolve
//! (e.g. towards a DAG) without breaking the UI, and so that the core stays free of serde.
//! Keep `app/src/lib/engine.ts` in sync with this file.
//!
//! Layer ids travel as JSON numbers: exact up to 2^53, far beyond what a session allocates.

use serde::{Deserialize, Serialize};
use slopshop_core::color::{ColorSpace, WORKING_SPACE};
use slopshop_core::view::{Viewport, ZoomStep};
use slopshop_core::{
    BlendMode, BlendSpace, Document, Edit, Layer, LayerContent, LayerId, LinearRgba, Session, Size,
};
use slopshop_io::export::{
    ExportError, ExportFormat, ExportFormatKind, ExportNotice, ExportSpec, ExrSample,
    JpegSubsampling, PngCompression, PngDepth, TiffCompression, TiffSample, WebpCompression,
    has_gray, supports_gray, supports_space,
};

/// Identity of an open document (one per tab). Ids are never reused, so the UI can tell
/// documents apart and ignore stale answers.
#[derive(Debug, Clone)]
pub struct DocumentMeta {
    pub id: u64,
    /// Display name (e.g. the file name). `None` for an untitled document.
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentView {
    pub id: u64,
    pub name: Option<String>,
    pub width: u32,
    pub height: u32,
    /// Identifier (e.g. `linear-srgb`), translated by the UI.
    pub working_space: &'static str,
    /// Where layers blend: `perceptual` or `linear` (ADR 0012).
    pub blend_space: &'static str,
    pub revision: u64,
    pub can_undo: bool,
    pub can_redo: bool,
    /// Bottom to top, like the core model.
    pub layers: Vec<LayerView>,
    pub warnings: Vec<&'static str>,
    /// The `.slop` file the document was opened from or saved to, if any.
    pub path: Option<String>,
    /// Changed since it was opened, created or last saved.
    pub dirty: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerView {
    pub id: u64,
    pub name: String,
    pub visible: bool,
    pub opacity: f32,
    /// Identifier of the blend mode (`normal`, `multiply`, `colorBurn`…), translated by the UI.
    pub blend_mode: &'static str,
    pub kind: &'static str,
    /// Changes when the layer's pixels change (the raster image's id; 0 for fills): the UI
    /// fetches a new thumbnail then.
    pub content_key: u64,
    /// A raster whose image has transparency (a mask can be made from it).
    pub has_alpha: bool,
    /// The layer's mask, if any (ADR 0014).
    pub mask: Option<MaskView>,
    /// Display swatch, sRGB-encoded RGBA in `[0, 1]` (explicitly converted, see `color`).
    pub swatch: [f32; 4],
    /// A group's layers, bottom to top (ADR 0015); empty for other layers.
    pub children: Vec<LayerView>,
    /// A group whose layers blend through it.
    pub pass_through: bool,
}

impl DocumentView {
    /// `warnings`: how the sources of the current layers were interpreted (identifiers such as
    /// `iccProfileIgnored`, translated by the UI).
    pub fn new(session: &Session, meta: &DocumentMeta, warnings: Vec<&'static str>) -> Self {
        let doc: &Document = session.document();
        Self {
            id: meta.id,
            name: meta.name.clone(),
            width: doc.size().width,
            height: doc.size().height,
            working_space: color_space_id(doc.working_space()),
            blend_space: doc.blend_space().id(),
            revision: doc.revision(),
            can_undo: session.can_undo(),
            can_redo: session.can_redo(),
            layers: doc.layers().iter().map(LayerView::new).collect(),
            warnings,
            path: None,
            dirty: false,
        }
    }
}

/// Why a document could not be saved (the error of `save_document`), translated by the UI
/// (`save.error.<code>`): a `.slop` file error code (`io`, `conflict`, `readOnly`…), `busy`
/// (a save of the document is already running), `documentClosed` or `internal`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveFailed {
    pub code: &'static str,
    /// Technical detail, shown inside the translated message.
    pub detail: String,
}

impl LayerView {
    fn new(layer: &Layer) -> Self {
        let (kind, swatch, content_key, has_alpha) = match &layer.content {
            LayerContent::Fill { color } => ("fill", color.working_to_srgb_encoded(), 0, false),
            LayerContent::Group { .. } => ("group", [0.0; 4], 0, false),
            LayerContent::Raster { image } => (
                "raster",
                image
                    .average_color(&WORKING_SPACE)
                    .working_to_srgb_encoded(),
                image.id().get(),
                image.format().layout.has_alpha(),
            ),
        };
        Self {
            id: layer.id.get(),
            name: layer.name.clone(),
            visible: layer.visible,
            opacity: layer.opacity,
            blend_mode: layer.blend_mode.id(),
            kind,
            content_key,
            has_alpha,
            mask: layer.mask.as_ref().map(|mask| MaskView {
                enabled: mask.enabled,
                content_key: mask.image.id().get(),
            }),
            swatch,
            children: layer
                .children()
                .map_or_else(Vec::new, |c| c.iter().map(LayerView::new).collect()),
            pass_through: matches!(
                layer.content,
                LayerContent::Group {
                    pass_through: true,
                    ..
                }
            ),
        }
    }
}

/// A layer's mask, as the layers panel shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaskView {
    pub enabled: bool,
    /// Changes when the mask's pixels change: time for a new mask thumbnail.
    pub content_key: u64,
}

/// Identifier of a color space without a name (e.g. read from an ICC profile).
pub const CUSTOM_SPACE: &str = "custom";

/// Stable identifiers, not display text: the UI owns all user-visible (translated) strings.
fn color_space_id(space: ColorSpace) -> &'static str {
    space.id().unwrap_or(CUSTOM_SPACE)
}

/// The named color spaces (those with a [`ColorSpace::id`]), in the order the UI lists them.
pub const NAMED_SPACES: [ColorSpace; 9] = [
    ColorSpace::SRGB,
    ColorSpace::DISPLAY_P3,
    ColorSpace::ADOBE_RGB,
    ColorSpace::PROPHOTO,
    ColorSpace::REC2020,
    ColorSpace::REC2100_PQ,
    ColorSpace::REC2100_HLG,
    ColorSpace::LINEAR_SRGB,
    ColorSpace::LINEAR_REC2020,
];

/// The named color space with this identifier.
pub fn named_space(id: &str) -> Option<ColorSpace> {
    NAMED_SPACES
        .into_iter()
        .find(|space| space.id() == Some(id))
}

/// Edits the UI can request. Translated into core [`Edit`]s; the engine validates them.
#[derive(Debug, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum EditRequest {
    /// Add a fill layer on top of the stack. `color` is sRGB-encoded RGBA in `[0, 1]`, as
    /// produced by UI color pickers; it is explicitly converted to the working space.
    AddFillLayer {
        name: String,
        color: [f32; 4],
    },
    RemoveLayer {
        id: u64,
    },
    SetLayerVisible {
        id: u64,
        visible: bool,
    },
    SetLayerOpacity {
        id: u64,
        opacity: f32,
    },
    RenameLayer {
        id: u64,
        name: String,
    },
    /// Move a layer so that it ends up at `index` (0 = bottom).
    MoveLayer {
        id: u64,
        index: usize,
    },
    /// `mode`: a blend mode identifier (`BlendMode::id`).
    SetLayerBlendMode {
        id: u64,
        mode: String,
    },
    /// `space`: `perceptual` or `linear`.
    SetBlendSpace {
        space: String,
    },
    SetLayerMaskEnabled {
        id: u64,
        enabled: bool,
    },
    RemoveLayerMask {
        id: u64,
    },
    /// Several edits as one: applied in order, all or none, undone together (e.g. an action on
    /// every selected layer).
    Batch {
        edits: Vec<EditRequest>,
    },
}

impl EditRequest {
    /// Build the core edit. Needs the session to allocate ids for new layers. Fails on an
    /// unknown blend mode or blend space identifier.
    pub fn into_edit(self, session: &mut Session) -> Result<Edit, String> {
        Ok(match self {
            EditRequest::AddFillLayer { name, color } => {
                let [r, g, b, a] = color;
                Edit::InsertLayer {
                    parent: None,
                    index: session.document().layers().len(),
                    layer: Layer {
                        id: session.allocate_layer_id(),
                        name,
                        visible: true,
                        opacity: 1.0,
                        blend_mode: BlendMode::Normal,
                        mask: None,
                        content: LayerContent::Fill {
                            color: LinearRgba::from_srgb_encoded_to_working(r, g, b, a),
                        },
                    },
                }
            }
            EditRequest::RemoveLayer { id } => Edit::RemoveLayer {
                id: LayerId::from_raw(id),
            },
            EditRequest::SetLayerVisible { id, visible } => Edit::SetLayerVisible {
                id: LayerId::from_raw(id),
                visible,
            },
            EditRequest::SetLayerOpacity { id, opacity } => Edit::SetLayerOpacity {
                id: LayerId::from_raw(id),
                opacity,
            },
            EditRequest::RenameLayer { id, name } => Edit::RenameLayer {
                id: LayerId::from_raw(id),
                name,
            },
            EditRequest::MoveLayer { id, index } => Edit::MoveLayer {
                parent: None,
                id: LayerId::from_raw(id),
                index,
            },
            EditRequest::SetLayerBlendMode { id, mode } => Edit::SetLayerBlendMode {
                id: LayerId::from_raw(id),
                mode: BlendMode::from_id(&mode).ok_or(format!("unknown blend mode {mode:?}"))?,
            },
            EditRequest::SetBlendSpace { space } => Edit::SetBlendSpace {
                space: BlendSpace::from_id(&space)
                    .ok_or(format!("unknown blend space {space:?}"))?,
            },
            EditRequest::SetLayerMaskEnabled { id, enabled } => Edit::SetLayerMaskEnabled {
                id: LayerId::from_raw(id),
                enabled,
            },
            EditRequest::RemoveLayerMask { id } => Edit::SetLayerMask {
                id: LayerId::from_raw(id),
                mask: None,
            },
            EditRequest::Batch { edits } => Edit::Batch(
                edits
                    .into_iter()
                    .map(|edit| edit.into_edit(session))
                    .collect::<Result<_, _>>()?,
            ),
        })
    }
}

/// View changes the UI can request. Positions and deltas are in viewport device pixels.
#[derive(Debug, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ViewRequest {
    /// Show the whole document, and keep doing so on resize.
    Fit,
    /// Set the zoom (1.0 = 100%) around the viewport center.
    SetZoom { zoom: f64 },
    /// Multiply the zoom around a point (wheel, pinch).
    ZoomBy { factor: f64, x: f64, y: f64 },
    /// Next preset zoom in or out, around a point or the viewport center.
    Step {
        zoom_in: bool,
        x: Option<f64>,
        y: Option<f64>,
    },
    /// Move the content by a delta (it follows the pointer).
    Pan { dx: f64, dy: f64 },
}

impl ViewRequest {
    pub fn apply(self, viewport: &mut Viewport, document: Size) {
        match self {
            ViewRequest::Fit => viewport.fit(document),
            ViewRequest::SetZoom { zoom } => viewport.set_zoom(document, zoom),
            ViewRequest::ZoomBy { factor, x, y } => viewport.zoom_by(document, [x, y], factor),
            ViewRequest::Step { zoom_in, x, y } => {
                let direction = if zoom_in { ZoomStep::In } else { ZoomStep::Out };
                viewport.step(document, x.zip(y).map(|(x, y)| [x, y]), direction);
            }
            ViewRequest::Pan { dx, dy } => viewport.pan(document, dx, dy),
        }
    }
}

/// The view after a change: enough for the UI to reproject the frame it is showing
/// (`document = origin + output / zoom`, output in device pixels) until the new one arrives.
/// Outcome of presenting a view directly to the window (native presentation).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentInfo {
    /// False when nothing was shown (window occluded, swapchain busy): present again later.
    pub presented: bool,
    pub revision: u64,
    /// 1.0 = 100%.
    pub zoom: f64,
    pub fit: bool,
    /// Engine-side time (composite + copy + present call), in ms.
    pub render_ms: f32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewInfo {
    /// 1.0 = 100%.
    pub zoom: f64,
    pub origin: [f64; 2],
    pub fit: bool,
}

impl ViewInfo {
    pub fn new(viewport: &Viewport) -> Self {
        Self {
            zoom: viewport.zoom(),
            origin: viewport.transform().origin,
            fit: viewport.is_fit(),
        }
    }
}

/// Binary frame sent to the UI: a fixed little-endian header followed by the RGBA8 sRGB
/// pixels (tightly packed rows, top to bottom). Keep `parseFrame` in `engine.ts` in sync.
///
/// | offset | type | field                                         |
/// | ------ | ---- | --------------------------------------------- |
/// | 0      | u32  | format version (2)                            |
/// | 4      | u32  | width                                         |
/// | 8      | u32  | height                                        |
/// | 12     | u32  | flags (bit 0: view is in fit mode)            |
/// | 16     | u64  | document revision rendered                    |
/// | 24     | f64  | zoom (1.0 = 100%)                             |
/// | 32     | f32  | engine render time in ms (GPU + readback)     |
/// | 36     | u32  | document id (low 32 bits)                     |
/// | 40     | f64  | view origin x (document px at output 0,0)     |
/// | 48     | f64  | view origin y                                 |
pub const FRAME_HEADER_LEN: usize = 56;
pub const FRAME_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy)]
pub struct FrameHeader {
    pub size: Size,
    pub fit: bool,
    pub revision: u64,
    pub zoom: f64,
    pub render_ms: f32,
    pub document_id: u64,
    pub origin: [f64; 2],
}

impl FrameHeader {
    pub fn to_bytes(self) -> [u8; FRAME_HEADER_LEN] {
        let mut bytes = [0u8; FRAME_HEADER_LEN];
        bytes[0..4].copy_from_slice(&FRAME_VERSION.to_le_bytes());
        bytes[4..8].copy_from_slice(&self.size.width.to_le_bytes());
        bytes[8..12].copy_from_slice(&self.size.height.to_le_bytes());
        bytes[12..16].copy_from_slice(&u32::from(self.fit).to_le_bytes());
        bytes[16..24].copy_from_slice(&self.revision.to_le_bytes());
        bytes[24..32].copy_from_slice(&self.zoom.to_le_bytes());
        bytes[32..36].copy_from_slice(&self.render_ms.to_le_bytes());
        // Truncation is intended: the UI only compares ids, and 2^32 opened documents is plenty.
        bytes[36..40].copy_from_slice(&(self.document_id as u32).to_le_bytes());
        bytes[40..48].copy_from_slice(&self.origin[0].to_le_bytes());
        bytes[48..56].copy_from_slice(&self.origin[1].to_le_bytes());
        bytes
    }
}

/// Export file formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormatId {
    Png,
    Tiff,
    Exr,
    Jpeg,
    Webp,
}

impl ExportFormatId {
    pub fn kind(self) -> ExportFormatKind {
        match self {
            ExportFormatId::Png => ExportFormatKind::Png,
            ExportFormatId::Tiff => ExportFormatKind::Tiff,
            ExportFormatId::Exr => ExportFormatKind::Exr,
            ExportFormatId::Jpeg => ExportFormatKind::Jpeg,
            ExportFormatId::Webp => ExportFormatKind::Webp,
        }
    }
}

/// Sample types of exported files: 8/16-bit integers, 16/32-bit floats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportSampleId {
    U8,
    U16,
    F16,
    F32,
}

/// Compression settings, per format: PNG `fast`/`small`, TIFF `none`/`deflate`/`lzw`, WebP
/// `lossy`/`lossless`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportCompressionId {
    Fast,
    Small,
    None,
    Deflate,
    Lzw,
    Lossy,
    Lossless,
}

/// JPEG chroma subsampling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportSubsamplingId {
    #[serde(rename = "444")]
    S444,
    #[serde(rename = "422")]
    S422,
    #[serde(rename = "420")]
    S420,
}

/// Export settings (see [`ExportSpec`]), flattened for the UI: which values are valid depends
/// on the format, and [`ExportSpecDto::to_spec`] rejects the combinations that make no sense.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSpecDto {
    pub format: ExportFormatId,
    /// PNG: `u8`, `u16`; TIFF: `u8`, `u16`, `f32`; EXR: `f32`, `f16`.
    pub sample: ExportSampleId,
    /// `null` for EXR, whose compression is fixed (lossless ZIP), and for JPEG.
    pub compression: Option<ExportCompressionId>,
    /// JPEG (1 to 100) and lossy WebP (0 to 100); `null` otherwise.
    pub quality: Option<u8>,
    /// JPEG only; `null` for the other formats.
    pub subsampling: Option<ExportSubsamplingId>,
    /// A named space ([`NAMED_SPACES`]), or [`CUSTOM_SPACE`] for the document's own unnamed
    /// space when [`default_spec`](slopshop_io::export::default_spec) picked it.
    pub space: String,
    pub keep_alpha: bool,
    /// The color transparency is flattened over when alpha is dropped: sRGB-encoded RGB in
    /// `[0, 1]`, as UI color pickers produce it; explicitly converted to the working space.
    pub matte: [f32; 3],
    /// Only applies to 8-bit samples.
    pub dither: bool,
    /// Gray samples (PNG, TIFF, JPEG): the luminance of the image in `space`.
    pub gray: bool,
}

impl ExportSpecDto {
    pub fn new(spec: &ExportSpec) -> Self {
        use ExportCompressionId as C;
        use ExportSampleId as S;
        let (mut quality, mut subsampling) = (None, None);
        let (format, sample, compression) = match spec.format {
            ExportFormat::Png { depth, compression } => (
                ExportFormatId::Png,
                match depth {
                    PngDepth::U8 => S::U8,
                    PngDepth::U16 => S::U16,
                },
                Some(match compression {
                    PngCompression::Fast => C::Fast,
                    PngCompression::Small => C::Small,
                }),
            ),
            ExportFormat::Tiff {
                sample,
                compression,
            } => (
                ExportFormatId::Tiff,
                match sample {
                    TiffSample::U8 => S::U8,
                    TiffSample::U16 => S::U16,
                    TiffSample::F32 => S::F32,
                },
                Some(match compression {
                    TiffCompression::None => C::None,
                    TiffCompression::Deflate => C::Deflate,
                    TiffCompression::Lzw => C::Lzw,
                }),
            ),
            ExportFormat::Exr { sample } => (
                ExportFormatId::Exr,
                match sample {
                    ExrSample::F32 => S::F32,
                    ExrSample::F16 => S::F16,
                },
                None,
            ),
            ExportFormat::Jpeg {
                quality: q,
                subsampling: s,
            } => {
                quality = Some(q);
                subsampling = Some(match s {
                    JpegSubsampling::S444 => ExportSubsamplingId::S444,
                    JpegSubsampling::S422 => ExportSubsamplingId::S422,
                    JpegSubsampling::S420 => ExportSubsamplingId::S420,
                });
                (ExportFormatId::Jpeg, S::U8, None)
            }
            ExportFormat::Webp { compression } => {
                let compression = match compression {
                    WebpCompression::Lossy { quality: q } => {
                        quality = Some(q);
                        C::Lossy
                    }
                    WebpCompression::Lossless => C::Lossless,
                };
                (ExportFormatId::Webp, S::U8, Some(compression))
            }
        };
        Self {
            format,
            sample,
            compression,
            quality,
            subsampling,
            space: color_space_id(spec.space).to_owned(),
            keep_alpha: spec.keep_alpha,
            // Rounded to 8-bit steps, as the UI's color picker shows it (and so that white
            // stays exactly 1 despite the matrices' rounding).
            matte: {
                let [r, g, b, _] = spec.matte.working_to_srgb_encoded();
                [r, g, b].map(|c| (c * 255.0).round() / 255.0)
            },
            dither: spec.dither,
            gray: spec.gray,
        }
    }

    /// The engine's settings. `custom` is the space [`CUSTOM_SPACE`] stands for (the
    /// document's own unnamed space), if any. Fails with [`ExportError::InvalidSpec`] on a
    /// sample type or compression the format does not have, or an unknown space, and with
    /// [`ExportError::UnsupportedSpace`] on a space the format cannot tag.
    pub fn to_spec(&self, custom: Option<ColorSpace>) -> Result<ExportSpec, ExportError> {
        use ExportCompressionId as C;
        use ExportSampleId as S;
        let invalid = |what: String| {
            ExportError::InvalidSpec(format!("{what} is not available for {:?}", self.format))
        };
        let sample = || invalid(format!("sample type {:?}", self.sample));
        let compression = || invalid(format!("compression {:?}", self.compression));
        let quality = || invalid(format!("quality {:?}", self.quality));
        if self.format != ExportFormatId::Jpeg && self.subsampling.is_some() {
            return Err(invalid("subsampling".to_owned()));
        }
        let lossy_webp = self.format == ExportFormatId::Webp && self.compression == Some(C::Lossy);
        if self.format != ExportFormatId::Jpeg && !lossy_webp && self.quality.is_some() {
            return Err(quality());
        }
        let format = match self.format {
            ExportFormatId::Png => ExportFormat::Png {
                depth: match self.sample {
                    S::U8 => PngDepth::U8,
                    S::U16 => PngDepth::U16,
                    S::F16 | S::F32 => return Err(sample()),
                },
                compression: match self.compression {
                    Some(C::Fast) => PngCompression::Fast,
                    Some(C::Small) => PngCompression::Small,
                    _ => return Err(compression()),
                },
            },
            ExportFormatId::Tiff => ExportFormat::Tiff {
                sample: match self.sample {
                    S::U8 => TiffSample::U8,
                    S::U16 => TiffSample::U16,
                    S::F32 => TiffSample::F32,
                    S::F16 => return Err(sample()),
                },
                compression: match self.compression {
                    Some(C::None) => TiffCompression::None,
                    Some(C::Deflate) => TiffCompression::Deflate,
                    Some(C::Lzw) => TiffCompression::Lzw,
                    _ => return Err(compression()),
                },
            },
            ExportFormatId::Exr => ExportFormat::Exr {
                sample: match (self.sample, self.compression) {
                    (S::F32, None) => ExrSample::F32,
                    (S::F16, None) => ExrSample::F16,
                    (_, None) => return Err(sample()),
                    (_, Some(_)) => return Err(compression()),
                },
            },
            ExportFormatId::Jpeg => {
                if self.sample != S::U8 {
                    return Err(sample());
                }
                if self.compression.is_some() {
                    return Err(compression());
                }
                if self.keep_alpha {
                    return Err(invalid("alpha".to_owned()));
                }
                ExportFormat::Jpeg {
                    quality: self
                        .quality
                        .filter(|q| (1..=100).contains(q))
                        .ok_or_else(quality)?,
                    subsampling: match self.subsampling {
                        Some(ExportSubsamplingId::S444) => JpegSubsampling::S444,
                        Some(ExportSubsamplingId::S422) => JpegSubsampling::S422,
                        Some(ExportSubsamplingId::S420) => JpegSubsampling::S420,
                        None => return Err(invalid("missing subsampling".to_owned())),
                    },
                }
            }
            ExportFormatId::Webp => {
                if self.sample != S::U8 {
                    return Err(sample());
                }
                ExportFormat::Webp {
                    compression: match self.compression {
                        Some(C::Lossless) => WebpCompression::Lossless,
                        Some(C::Lossy) => WebpCompression::Lossy {
                            quality: self.quality.filter(|q| *q <= 100).ok_or_else(quality)?,
                        },
                        _ => return Err(compression()),
                    },
                }
            }
        };
        let space = if self.space == CUSTOM_SPACE {
            custom.ok_or_else(|| {
                ExportError::InvalidSpec("the document has no custom color space".to_owned())
            })?
        } else {
            named_space(&self.space).ok_or_else(|| {
                ExportError::InvalidSpec(format!("unknown color space {:?}", self.space))
            })?
        };
        let kind = format.kind();
        if self.gray && !has_gray(kind) {
            return Err(ExportError::InvalidSpec(format!(
                "{kind:?} export has no gray samples"
            )));
        }
        let taggable = if self.gray {
            supports_gray(kind, &space)
        } else {
            supports_space(kind, &space)
        };
        if !taggable {
            return Err(ExportError::UnsupportedSpace(space));
        }
        let [r, g, b] = self.matte;
        if !self.matte.iter().all(|c| (0.0..=1.0).contains(c)) {
            return Err(ExportError::InvalidSpec(format!(
                "matte {:?} is not an sRGB color in [0, 1]",
                self.matte
            )));
        }
        Ok(ExportSpec {
            format,
            space,
            keep_alpha: self.keep_alpha,
            matte: LinearRgba::from_srgb_encoded_to_working(r, g, b, 1.0),
            dither: self.dither,
            gray: self.gray,
            // The document's, set by the caller: the UI does not choose it.
            blend_space: BlendSpace::default(),
        })
    }
}

/// An export job has started (`export-started`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportStarted {
    pub id: u64,
    pub document_id: u64,
    pub path: String,
    /// File name, for messages.
    pub name: String,
}

/// How far an export has gone (`export-progress`), in rows.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportProgress {
    pub id: u64,
    pub done: u64,
    pub total: u64,
}

/// An entry of an export report, translated by the UI (`export.report.<id>`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportNoticeView {
    pub id: &'static str,
    /// Number of samples concerned (pixels for `alphaFlattened` and `colorDiscarded`), for the
    /// notices that count something.
    pub count: Option<u64>,
}

impl ExportNoticeView {
    pub fn new(notice: ExportNotice) -> Self {
        Self {
            id: notice.id(),
            count: notice.count(),
        }
    }
}

/// An export has written its file (`export-finished`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportFinished {
    pub id: u64,
    pub path: String,
    /// What was lost or changed on the way (empty when nothing was).
    pub notices: Vec<ExportNoticeView>,
}

/// Why an export failed: the `export-failed` event (with the job id), or the error of
/// `export_document` when it could not start a job (without).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportFailed {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    /// Stable identifier translated by the UI (`export.error.<code>`): an
    /// [`ExportError::code`] (`cancelled` for a cancelled export), `documentClosed` or
    /// `internal`.
    pub code: &'static str,
    /// Technical detail (system message, size, …), shown inside the translated message.
    pub detail: String,
}

impl ExportFailed {
    pub fn new(id: Option<u64>, error: &ExportError) -> Self {
        Self {
            id,
            code: error.code(),
            detail: error.to_string(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuInfo {
    pub name: String,
    pub backend: String,
    pub device_type: String,
    pub driver: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use slopshop_core::color::{RgbPrimaries, TransferFunction};
    use slopshop_io::export::{WHITE_MATTE, default_spec};

    fn dto(json: &str) -> ExportSpecDto {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn named_spaces_have_distinct_ids() {
        let ids: Vec<&str> = NAMED_SPACES.iter().filter_map(ColorSpace::id).collect();
        assert_eq!(
            ids.len(),
            NAMED_SPACES.len(),
            "every listed space has an id"
        );
        for (i, id) in ids.iter().enumerate() {
            assert!(!ids[i + 1..].contains(id), "{id} listed twice");
            assert_eq!(named_space(id).and_then(|s| s.id()), Some(*id));
        }
        assert_eq!(named_space(CUSTOM_SPACE), None);
        assert_eq!(named_space("bogus"), None);
    }

    #[test]
    fn export_specs_travel_as_ids() {
        let json = r#"{"format":"png","sample":"u16","compression":"small","quality":null,"subsampling":null,"space":"display-p3","keepAlpha":true,"matte":[1.0,1.0,1.0],"dither":false,"gray":false}"#;
        let spec = dto(json).to_spec(None).unwrap();
        assert_eq!(
            spec,
            ExportSpec {
                format: ExportFormat::Png {
                    depth: PngDepth::U16,
                    compression: PngCompression::Small,
                },
                space: ColorSpace::DISPLAY_P3,
                keep_alpha: true,
                matte: WHITE_MATTE,
                dither: false,
                gray: false,
                blend_space: BlendSpace::default(),
            }
        );
        assert_eq!(
            serde_json::to_string(&ExportSpecDto::new(&spec)).unwrap(),
            json
        );

        let exr = dto(
            r#"{"format":"exr","sample":"f16","compression":null,"quality":null,"subsampling":null,"space":"linear-rec2020","keepAlpha":false,"matte":[1.0,1.0,1.0],"dither":false,"gray":false}"#,
        );
        assert_eq!(
            exr.to_spec(None).unwrap().format,
            ExportFormat::Exr {
                sample: ExrSample::F16
            }
        );
    }

    #[test]
    fn default_specs_round_trip() {
        let document = Document::new(Size::new(8, 8));
        for format in [
            ExportFormatId::Png,
            ExportFormatId::Tiff,
            ExportFormatId::Exr,
            ExportFormatId::Jpeg,
            ExportFormatId::Webp,
        ] {
            let spec = default_spec(format.kind(), &document);
            let dto = ExportSpecDto::new(&spec);
            assert_eq!(dto.format, format);
            assert_eq!(dto.to_spec(None).unwrap(), spec, "{format:?}");
        }
        let tiff = ExportSpec {
            format: ExportFormat::Tiff {
                sample: TiffSample::F32,
                compression: TiffCompression::Lzw,
            },
            space: ColorSpace::LINEAR_REC2020,
            keep_alpha: true,
            matte: WHITE_MATTE,
            dither: true,
            gray: false,
            blend_space: BlendSpace::default(),
        };
        assert_eq!(ExportSpecDto::new(&tiff).to_spec(None).unwrap(), tiff);
    }

    #[test]
    fn invalid_export_specs_are_rejected() {
        let base = dto(
            r#"{"format":"png","sample":"u8","compression":"fast","quality":null,"subsampling":null,"space":"srgb","keepAlpha":true,"matte":[1.0,1.0,1.0],"dither":true,"gray":false}"#,
        );
        let with = |change: &dyn Fn(&mut ExportSpecDto)| {
            let mut dto = base.clone();
            change(&mut dto);
            dto.to_spec(None).map(|_| ()).map_err(|e| e.code())
        };
        use ExportCompressionId as C;
        use ExportSampleId as S;
        assert_eq!(with(&|_| {}), Ok(()));
        let invalid = Err("invalidSpec");
        assert_eq!(with(&|d| d.sample = S::F32), invalid, "PNG float");
        assert_eq!(with(&|d| d.compression = Some(C::Lzw)), invalid);
        assert_eq!(with(&|d| d.compression = None), invalid);
        assert_eq!(
            with(&|d| {
                d.format = ExportFormatId::Tiff;
                d.compression = Some(C::Deflate);
                d.sample = S::F16;
            }),
            invalid,
            "TIFF half"
        );
        assert_eq!(
            with(&|d| d.format = ExportFormatId::Tiff),
            invalid,
            "TIFF with a PNG compression"
        );
        assert_eq!(
            with(&|d| {
                d.format = ExportFormatId::Exr;
                d.space = "linear-srgb".to_owned();
            }),
            invalid,
            "EXR has no compression setting"
        );
        assert_eq!(
            with(&|d| {
                d.format = ExportFormatId::Exr;
                d.space = "linear-srgb".to_owned();
                d.compression = None;
            }),
            invalid,
            "EXR 8-bit"
        );
        assert_eq!(with(&|d| d.space = "bogus".to_owned()), invalid);
        assert_eq!(
            with(&|d| d.matte = [1.5, 0.0, 0.0]),
            invalid,
            "matte above 1"
        );
        assert_eq!(
            with(&|d| d.matte = [f32::NAN, 0.0, 0.0]),
            invalid,
            "NaN matte"
        );
        assert_eq!(
            with(&|d| d.space = CUSTOM_SPACE.to_owned()),
            invalid,
            "no custom space to stand for"
        );
        assert_eq!(
            with(&|d| {
                d.format = ExportFormatId::Exr;
                d.sample = S::F32;
                d.compression = None;
            }),
            Err("unsupportedSpace"),
            "EXR is linear only"
        );
        assert!(
            serde_json::from_str::<ExportSpecDto>(
                r#"{"format":"gif","sample":"u8","compression":null,"quality":null,"subsampling":null,"space":"srgb","keepAlpha":false,"matte":[1.0,1.0,1.0],"dither":false,"gray":false}"#
            )
            .is_err()
        );
        assert_eq!(with(&|d| d.gray = true), Ok(()), "gray PNG");
        assert_eq!(
            with(&|d| {
                d.gray = true;
                d.space = "rec2100-pq".to_owned();
            }),
            Err("unsupportedSpace"),
            "gray PQ"
        );
        assert_eq!(
            with(&|d| {
                d.format = ExportFormatId::Webp;
                d.compression = Some(C::Lossless);
                d.gray = true;
            }),
            invalid,
            "gray WebP"
        );
        assert_eq!(with(&|d| d.quality = Some(90)), invalid, "quality on PNG");
        assert_eq!(
            with(&|d| d.subsampling = Some(ExportSubsamplingId::S420)),
            invalid,
            "subsampling on PNG"
        );
        let jpeg = |change: &dyn Fn(&mut ExportSpecDto)| {
            with(&|d| {
                d.format = ExportFormatId::Jpeg;
                d.compression = None;
                d.keep_alpha = false;
                d.quality = Some(90);
                d.subsampling = Some(ExportSubsamplingId::S444);
                change(d);
            })
        };
        assert_eq!(jpeg(&|_| {}), Ok(()));
        assert_eq!(jpeg(&|d| d.keep_alpha = true), invalid, "JPEG with alpha");
        assert_eq!(jpeg(&|d| d.quality = Some(0)), invalid, "JPEG quality 0");
        assert_eq!(
            jpeg(&|d| d.quality = Some(101)),
            invalid,
            "JPEG quality 101"
        );
        assert_eq!(jpeg(&|d| d.quality = None), invalid, "JPEG without quality");
        assert_eq!(
            jpeg(&|d| d.subsampling = None),
            invalid,
            "JPEG without subsampling"
        );
        assert_eq!(jpeg(&|d| d.sample = S::U16), invalid, "JPEG 16-bit");
        assert_eq!(
            jpeg(&|d| d.compression = Some(C::Fast)),
            invalid,
            "JPEG compression"
        );
        assert_eq!(
            jpeg(&|d| d.space = "rec2100-pq".to_owned()),
            Err("unsupportedSpace"),
            "JPEG PQ"
        );
        let webp = |change: &dyn Fn(&mut ExportSpecDto)| {
            with(&|d| {
                d.format = ExportFormatId::Webp;
                d.compression = Some(C::Lossy);
                d.quality = Some(0);
                change(d);
            })
        };
        assert_eq!(webp(&|_| {}), Ok(()), "lossy quality 0");
        assert_eq!(
            webp(&|d| {
                d.compression = Some(C::Lossless);
                d.quality = None;
            }),
            Ok(())
        );
        assert_eq!(
            webp(&|d| d.compression = Some(C::Lossless)),
            invalid,
            "quality on lossless"
        );
        assert_eq!(
            webp(&|d| d.quality = None),
            invalid,
            "lossy without quality"
        );
        assert_eq!(webp(&|d| d.quality = Some(101)), invalid, "quality 101");
        assert_eq!(webp(&|d| d.compression = None), invalid, "no compression");
        assert_eq!(
            webp(&|d| d.compression = Some(C::Fast)),
            invalid,
            "PNG compression"
        );
        assert_eq!(
            webp(&|d| d.subsampling = Some(ExportSubsamplingId::S420)),
            invalid
        );
        assert_eq!(webp(&|d| d.sample = S::U16), invalid, "16-bit WebP");
        assert_eq!(
            with(&|d| d.compression = Some(C::Lossless)),
            invalid,
            "lossless on PNG"
        );
    }

    #[test]
    fn the_custom_space_id_stands_for_the_given_space() {
        let unnamed = ColorSpace {
            primaries: RgbPrimaries::ADOBE_RGB,
            transfer: TransferFunction::Srgb,
        };
        assert_eq!(unnamed.id(), None);
        let spec = ExportSpec {
            format: ExportFormat::Tiff {
                sample: TiffSample::U16,
                compression: TiffCompression::Deflate,
            },
            space: unnamed,
            keep_alpha: false,
            matte: WHITE_MATTE,
            dither: true,
            gray: false,
            blend_space: BlendSpace::default(),
        };
        let dto = ExportSpecDto::new(&spec);
        assert_eq!(dto.space, CUSTOM_SPACE);
        assert_eq!(dto.to_spec(Some(unnamed)).unwrap(), spec);
    }

    #[test]
    fn export_events_serialize_for_the_ui() {
        let notices: Vec<ExportNoticeView> = [
            ExportNotice::ClippedHigh(12),
            ExportNotice::BigTiff,
            ExportNotice::AlphaFlattened(7),
        ]
        .into_iter()
        .map(ExportNoticeView::new)
        .collect();
        let json = serde_json::to_string(&notices).unwrap();
        assert_eq!(
            json,
            r#"[{"id":"clippedHigh","count":12},{"id":"bigTiff","count":null},{"id":"alphaFlattened","count":7}]"#
        );
        let failed = ExportFailed::new(Some(3), &ExportError::Cancelled);
        assert_eq!(
            serde_json::to_string(&failed).unwrap(),
            r#"{"id":3,"code":"cancelled","detail":"cancelled"}"#
        );
        let refused = ExportFailed::new(None, &ExportError::InvalidSpec("x".to_owned()));
        assert_eq!(
            serde_json::to_string(&refused).unwrap(),
            r#"{"code":"invalidSpec","detail":"x"}"#
        );
    }
}
