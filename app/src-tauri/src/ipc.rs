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
use slopshop_core::{Document, Edit, Layer, LayerContent, LayerId, LinearRgba, Session, Size};
use slopshop_io::export::{
    ExportError, ExportFormat, ExportFormatKind, ExportNotice, ExportSpec, ExrSample,
    PngCompression, PngDepth, TiffCompression, TiffSample, supports_space,
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
    pub revision: u64,
    pub can_undo: bool,
    pub can_redo: bool,
    /// Bottom to top, like the core model.
    pub layers: Vec<LayerView>,
    pub warnings: Vec<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerView {
    pub id: u64,
    pub name: String,
    pub visible: bool,
    pub opacity: f32,
    pub kind: &'static str,
    /// Display swatch, sRGB-encoded RGBA in `[0, 1]` (explicitly converted, see `color`).
    pub swatch: [f32; 4],
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
            revision: doc.revision(),
            can_undo: session.can_undo(),
            can_redo: session.can_redo(),
            layers: doc.layers().iter().map(LayerView::new).collect(),
            warnings,
        }
    }
}

impl LayerView {
    fn new(layer: &Layer) -> Self {
        let (kind, swatch) = match &layer.content {
            LayerContent::Fill { color } => ("fill", color.working_to_srgb_encoded()),
            LayerContent::Raster { image } => (
                "raster",
                image
                    .average_color(&WORKING_SPACE)
                    .working_to_srgb_encoded(),
            ),
        };
        Self {
            id: layer.id.get(),
            name: layer.name.clone(),
            visible: layer.visible,
            opacity: layer.opacity,
            kind,
            swatch,
        }
    }
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
}

impl EditRequest {
    /// Build the core edit. Needs the session to allocate ids for new layers.
    pub fn into_edit(self, session: &mut Session) -> Edit {
        match self {
            EditRequest::AddFillLayer { name, color } => {
                let [r, g, b, a] = color;
                Edit::InsertLayer {
                    index: session.document().layers().len(),
                    layer: Layer {
                        id: session.allocate_layer_id(),
                        name,
                        visible: true,
                        opacity: 1.0,
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
                id: LayerId::from_raw(id),
                index,
            },
        }
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
}

impl ExportFormatId {
    pub fn kind(self) -> ExportFormatKind {
        match self {
            ExportFormatId::Png => ExportFormatKind::Png,
            ExportFormatId::Tiff => ExportFormatKind::Tiff,
            ExportFormatId::Exr => ExportFormatKind::Exr,
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

/// Compression settings, per format: PNG `fast`/`small`, TIFF `none`/`deflate`/`lzw`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportCompressionId {
    Fast,
    Small,
    None,
    Deflate,
    Lzw,
}

/// Export settings (see [`ExportSpec`]), flattened for the UI: which values are valid depends
/// on the format, and [`ExportSpecDto::to_spec`] rejects the combinations that make no sense.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSpecDto {
    pub format: ExportFormatId,
    /// PNG: `u8`, `u16`; TIFF: `u8`, `u16`, `f32`; EXR: `f32`, `f16`.
    pub sample: ExportSampleId,
    /// `null` for EXR, whose compression is fixed (lossless ZIP).
    pub compression: Option<ExportCompressionId>,
    /// A named space ([`NAMED_SPACES`]), or [`CUSTOM_SPACE`] for the document's own unnamed
    /// space when [`default_spec`](slopshop_io::export::default_spec) picked it.
    pub space: String,
    pub keep_alpha: bool,
    /// Only applies to 8-bit samples.
    pub dither: bool,
}

impl ExportSpecDto {
    pub fn new(spec: &ExportSpec) -> Self {
        use ExportCompressionId as C;
        use ExportSampleId as S;
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
        };
        Self {
            format,
            sample,
            compression,
            space: color_space_id(spec.space).to_owned(),
            keep_alpha: spec.keep_alpha,
            dither: spec.dither,
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
        if !supports_space(format.kind(), &space) {
            return Err(ExportError::UnsupportedSpace(space));
        }
        Ok(ExportSpec {
            format,
            space,
            keep_alpha: self.keep_alpha,
            dither: self.dither,
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
    /// Number of samples concerned, for the notices that count something.
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
    use slopshop_io::export::default_spec;

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
        let json = r#"{"format":"png","sample":"u16","compression":"small","space":"display-p3","keepAlpha":true,"dither":false}"#;
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
                dither: false,
            }
        );
        assert_eq!(
            serde_json::to_string(&ExportSpecDto::new(&spec)).unwrap(),
            json
        );

        let exr = dto(
            r#"{"format":"exr","sample":"f16","compression":null,"space":"linear-rec2020","keepAlpha":false,"dither":false}"#,
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
            dither: true,
        };
        assert_eq!(ExportSpecDto::new(&tiff).to_spec(None).unwrap(), tiff);
    }

    #[test]
    fn invalid_export_specs_are_rejected() {
        let base = dto(
            r#"{"format":"png","sample":"u8","compression":"fast","space":"srgb","keepAlpha":true,"dither":true}"#,
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
                r#"{"format":"jpeg","sample":"u8","compression":null,"space":"srgb","keepAlpha":false,"dither":false}"#
            )
            .is_err()
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
            dither: true,
        };
        let dto = ExportSpecDto::new(&spec);
        assert_eq!(dto.space, CUSTOM_SPACE);
        assert_eq!(dto.to_spec(Some(unnamed)).unwrap(), spec);
    }

    #[test]
    fn export_events_serialize_for_the_ui() {
        let notices: Vec<ExportNoticeView> = [ExportNotice::ClippedHigh(12), ExportNotice::BigTiff]
            .into_iter()
            .map(ExportNoticeView::new)
            .collect();
        let json = serde_json::to_string(&notices).unwrap();
        assert_eq!(
            json,
            r#"[{"id":"clippedHigh","count":12},{"id":"bigTiff","count":null}]"#
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
