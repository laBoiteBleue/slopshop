//! IPC data types: the contract between the UI and the engine.
//!
//! These DTOs are deliberately separate from the core model so that the engine can evolve
//! (e.g. towards a DAG) without breaking the UI, and so that the core stays free of serde.
//! Keep `app/src/lib/engine.ts` in sync with this file.
//!
//! Layer ids travel as JSON numbers: exact up to 2^53, far beyond what a session allocates.

use serde::{Deserialize, Serialize};
use slopshop_core::adjust::Adjustment;
use slopshop_core::color::{ColorSpace, WORKING_SPACE};
use slopshop_core::curve::Curve;
use slopshop_core::stack::Entry;
use slopshop_core::view::{Viewport, ZoomStep};
use slopshop_core::{
    Arrange, BlendMode, BlendSpace, Document, Edit, ImageTurn, Layer, LayerContent, LayerId,
    LinearRgba, Session, Size,
};
use slopshop_io::export::{
    AvifDepth, ExportError, ExportFormat, ExportFormatKind, ExportNotice, ExportSpec, ExrSample,
    Jpeg2000Compression, JpegSubsampling, PngCompression, PngDepth, PsdDepth, TgaCompression,
    TiffCompression, TiffSample, WebpCompression, has_gray, supports_gray, supports_space,
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
    /// Pixels per inch (ADR 0028).
    pub resolution: f64,
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
    /// Identity of the selection's mask (ADR 0024): a new value means a new outline; `None`
    /// when nothing is selected.
    pub selection_key: Option<u64>,
    /// Select > Reselect has a selection to bring back.
    pub can_reselect: bool,
    /// The view shows Quick Mask (ADR 0024): view state, not part of the document.
    pub quick_mask: bool,
    /// Quick Mask's overlay opacity, percent.
    pub quick_mask_opacity: u8,
    /// The selections saved by name (Select > Save Selection), in the order they were saved.
    pub saved_selections: Vec<SavedSelectionView>,
}

/// A selection saved by name: what Select > Load Selection lists.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedSelectionView {
    pub id: u64,
    pub name: String,
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
    /// An adjustment layer's adjustment (ADR 0020), for the Properties panel.
    pub adjustment: Option<AdjustmentView>,
    /// A group's layers, bottom to top (ADR 0015); empty for other layers.
    pub children: Vec<LayerView>,
    /// A group whose layers blend through it.
    pub pass_through: bool,
    /// Clipped to the layer below it (ADR 0016).
    pub clipped: bool,
    /// From the layer's content to its parent (ADR 0017): `[a, b, c, d, e, f]`, a point
    /// `(x, y)` going to `(a·x + c·y + e, b·x + d·y + f)`.
    pub transform: [f64; 6],
    /// Its pixels or its mask carry paint (ADR 0027): Layer > Delete Paint removes it.
    pub painted: bool,
    /// What was applied to a raster layer's pixels (ADR 0029), bottom to top.
    pub entries: Vec<EntryView>,
}

/// An entry of a raster layer's stack (ADR 0029).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryView {
    /// `paint` or `effect`.
    pub kind: &'static str,
    /// An effect's adjustment (`Adjustment::id`), translated by the UI.
    pub adjustment: Option<&'static str>,
    /// How many times an effect of this kind was applied in a row (1 for paint).
    pub count: usize,
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
            resolution: doc.resolution(),
            revision: doc.revision(),
            can_undo: session.can_undo(),
            can_redo: session.can_redo(),
            layers: doc.layers().iter().map(LayerView::new).collect(),
            warnings,
            path: None,
            dirty: false,
            selection_key: doc.selection().map(|s| s.image().id().get()),
            can_reselect: false,
            quick_mask: false,
            quick_mask_opacity: 50,
            saved_selections: doc
                .saved_selections()
                .iter()
                .map(|s| SavedSelectionView {
                    id: s.id.get(),
                    name: s.name.clone(),
                })
                .collect(),
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
            LayerContent::Adjustment { .. } => ("adjustment", [0.0; 4], 0, false),
            // Never waits for a stack's pixels (ADR 0029): the original's color meanwhile.
            LayerContent::Raster { image, .. } => (
                "raster",
                image
                    .ready_image()
                    .or_else(|| layer.content.original())
                    .map_or([0.0; 4], |i| {
                        i.average_color(&WORKING_SPACE).working_to_srgb_encoded()
                    }),
                image.key(),
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
            adjustment: match &layer.content {
                LayerContent::Adjustment { adjustment } => Some(AdjustmentView {
                    id: adjustment.id(),
                    values: adjustment.params().to_vec(),
                    curves: adjustment
                        .curves()
                        .map(|curves| curves.map(|c| c.points().to_vec())),
                    curve_samples: adjustment.curves().map(|curves| {
                        curves.map(|c| {
                            (0..=CURVE_SAMPLES)
                                .map(|i| c.value(i as f64 / CURVE_SAMPLES as f64) as f32)
                                .collect()
                        })
                    }),
                    gradient: match adjustment {
                        Adjustment::GradientMap { gradient, .. } => Some(gradient_stops(gradient)),
                        _ => None,
                    },
                }),
                _ => None,
            },
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
            clipped: layer.clipped,
            transform: layer.transform.to_array(),
            painted: layer.is_painted(),
            entries: match &layer.content {
                LayerContent::Raster {
                    stack: Some(stack), ..
                } => stack
                    .entries()
                    .iter()
                    .map(|entry| match entry {
                        Entry::Paint(_) => EntryView {
                            kind: "paint",
                            adjustment: None,
                            count: 1,
                        },
                        Entry::Effect(effect) => EntryView {
                            kind: "effect",
                            adjustment: Some(effect.kind()),
                            count: effect.steps().len(),
                        },
                    })
                    .collect(),
                _ => Vec::new(),
            },
        }
    }
}

/// An adjustment layer's adjustment: its identifier and its parameters (all
/// `PARAM_COUNT` of them, in `Adjustment::params` order).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdjustmentView {
    pub id: &'static str,
    pub values: Vec<f32>,
    /// Curves' points `[input, output]` on 0–255: composite, red, green, blue.
    pub curves: Option<[Vec<[u8; 2]>; 4]>,
    /// Each curve's output at `i / CURVE_SAMPLES` (0 to 1), for the editor to draw.
    pub curve_samples: Option<[Vec<f32>; 4]>,
    /// Gradient Map's stops `[location 0–4096, r, g, b]` (not reversed: `values[0]` says).
    pub gradient: Option<Vec<[u16; 4]>>,
}

/// Intervals of [`AdjustmentView::curve_samples`].
const CURVE_SAMPLES: usize = 128;

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
pub(crate) fn color_space_id(space: ColorSpace) -> &'static str {
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
    /// Add a fill layer at `index` among the layers of `parent` (absent: the top level; no
    /// index: on top). `color` is sRGB-encoded RGBA in `[0, 1]`, as produced by UI color
    /// pickers; it is explicitly converted to the working space.
    AddFillLayer {
        name: String,
        color: [f32; 4],
        #[serde(default)]
        parent: Option<u64>,
        #[serde(default)]
        index: Option<usize>,
    },
    /// A fill layer's color, sRGB-encoded RGBA in `[0, 1]` as `AddFillLayer`'s.
    SetFillColor {
        id: u64,
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
    /// Move a layer so that it ends up at `index` (0 = bottom) among the layers of `parent`
    /// (absent: the top level).
    MoveLayer {
        id: u64,
        #[serde(default)]
        parent: Option<u64>,
        index: usize,
    },
    /// Move several layers together (keeping their order) into `parent` (absent: the top
    /// level), at `index` among its layers that do not move.
    MoveLayers {
        ids: Vec<u64>,
        #[serde(default)]
        parent: Option<u64>,
        index: usize,
    },
    /// A new empty pass-through group at `index` among the layers of `parent`.
    AddGroup {
        name: String,
        #[serde(default)]
        parent: Option<u64>,
        index: usize,
    },
    /// A new adjustment layer (ADR 0020) at its neutral parameters, at `index` among the layers
    /// of `parent` (absent: the top level). `adjustment`: `exposure`, `hueSaturation`, `levels`.
    AddAdjustmentLayer {
        name: String,
        adjustment: String,
        #[serde(default)]
        parent: Option<u64>,
        index: usize,
    },
    /// An adjustment layer's parameters (`values`, in `Adjustment::params` order, at most
    /// `PARAM_COUNT`; missing ones are 0).
    SetAdjustment {
        id: u64,
        adjustment: String,
        values: Vec<f32>,
        /// Curves' points (`AdjustmentView::curves`); required for `curves`, ignored
        /// otherwise.
        #[serde(default)]
        curves: Option<Vec<Vec<[u8; 2]>>>,
        /// Gradient Map's stops (`AdjustmentView::gradient`); required for `gradientMap`.
        #[serde(default)]
        gradient: Option<Vec<[u16; 4]>>,
    },
    /// A new empty layer to paint on (Layer > New > Layer, ADR 0027): canvas-sized, 8-bit
    /// sRGB, transparent, at `index` among the layers of `parent` (absent: the top level).
    AddEmptyLayer {
        name: String,
        #[serde(default)]
        parent: Option<u64>,
        index: usize,
    },
    /// Remove the paint of layers and of the layers inside them, pixels and masks (Layer >
    /// Delete Paint, ADR 0027).
    DeletePaint {
        ids: Vec<u64>,
    },
    /// Delete entry `index` (bottom to top) of a raster layer's stack (ADR 0029).
    DeleteStackEntry {
        id: u64,
        index: usize,
    },
    /// Image > Adjustments (ADR 0029): the adjustment applied to the visible raster layers of
    /// `ids`, within the selection (values and curves as `SetAdjustment`).
    ApplyEffect {
        ids: Vec<u64>,
        adjustment: String,
        values: Vec<f32>,
        #[serde(default)]
        curves: Option<Vec<Vec<[u8; 2]>>>,
        #[serde(default)]
        gradient: Option<Vec<[u16; 4]>>,
    },
    /// Image > Auto Tone, Auto Contrast and Auto Color (`correction`: `tone`, `contrast` or
    /// `color`):
    /// the visible image analyzed, the Levels found applied as `ApplyEffect` does to `ids`.
    /// Nothing to analyze or to change: nothing done.
    AutoLevels {
        ids: Vec<u64>,
        correction: String,
    },
    /// What Image > Adjustments will do, shown while its dialog is open: above each layer it
    /// applies to, an adjustment layer at its neutral settings clipped to it, the selection as
    /// its mask. Meant for a live gesture that is cancelled (then `ApplyEffect`).
    PreviewEffect {
        ids: Vec<u64>,
        adjustment: String,
    },
    /// Put layers into a new group in the place of the topmost of them (Layer > Group Layers).
    GroupLayers {
        ids: Vec<u64>,
        name: String,
    },
    /// Replace the groups among `ids` by their layers (Layer > Ungroup Layers); the other
    /// layers are left alone.
    Ungroup {
        ids: Vec<u64>,
    },
    /// Move layers within their groups (Layer > Arrange): `arrange` is `front`, `forward`,
    /// `backward` or `back`.
    ArrangeLayers {
        ids: Vec<u64>,
        arrange: String,
    },
    /// Copies of layers, each right above its original (Layer > Duplicate Layer). A copy is
    /// named by `name_format` with `{name}` replaced by the original's name (the UI translates
    /// it, e.g. `{name} copy`).
    DuplicateLayers {
        ids: Vec<u64>,
        name_format: String,
    },
    /// Copies of layers (as `DuplicateLayers`), then `matrix` applied to the copies (as
    /// `TransformLayers`): Photoshop's Duplicate and Transform Again (Alt+Shift+Ctrl+T), one
    /// undo entry.
    DuplicateTransformLayers {
        ids: Vec<u64>,
        name_format: String,
        matrix: [f64; 6],
    },
    SetGroupPassThrough {
        id: u64,
        pass_through: bool,
    },
    /// Clip a layer to the one below it, or release it (ADR 0016).
    SetLayerClipped {
        id: u64,
        clipped: bool,
    },
    /// Move layers by whole document pixels (the Move tool, ADR 0017); a group moves whole.
    TranslateLayers {
        ids: Vec<u64>,
        dx: i64,
        dy: i64,
    },
    /// Apply `matrix` (`[a, b, c, d, e, f]`, a map of the document's space) to layers on top
    /// of their transforms (Free Transform, ADR 0018); a group transforms whole.
    TransformLayers {
        ids: Vec<u64>,
        matrix: [f64; 6],
    },
    /// Image > Image Size: the whole image resampled to `width` × `height` (ADR 0018).
    ResizeImage {
        width: u32,
        height: u32,
        /// The new resolution, pixels per inch (ADR 0028), in the same undo entry; absent: kept.
        #[serde(default)]
        resolution: Option<f64>,
    },
    /// Image > Canvas Size: the canvas resized, the image kept at `anchor` (each in [0, 1]:
    /// 0 left/top, 0.5 center, 1 right/bottom).
    CanvasSize {
        width: u32,
        height: u32,
        anchor: [f64; 2],
    },
    /// The Crop tool: keep `width` × `height` pixels from (`x`, `y`) (it may extend past the
    /// canvas); nothing is deleted.
    Crop {
        x: i64,
        y: i64,
        width: i64,
        height: i64,
    },
    /// Image > Image Rotation: `turn` is `clockwise`, `counterClockwise`, `halfTurn`,
    /// `flipHorizontal` or `flipVertical`.
    RotateImage {
        turn: String,
    },
    /// Image > Image Rotation > Arbitrary: the whole image turned by `degrees` clockwise, the
    /// canvas grown to hold it.
    RotateImageBy {
        degrees: f64,
    },
    /// Image > Reveal All: the canvas grown to every layer's pixels (nothing when they are all
    /// on it).
    RevealAll,
    /// Image > Trim: the margins of `basis` (`transparent`, `topLeft` or `bottomRight`) taken
    /// off the sides asked (nothing when there are none).
    Trim {
        basis: String,
        top: bool,
        bottom: bool,
        left: bool,
        right: bool,
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
            EditRequest::AddFillLayer {
                name,
                color,
                parent,
                index,
            } => {
                let [r, g, b, a] = color;
                let parent = parent.map(LayerId::from_raw);
                let index = match index {
                    Some(index) => index,
                    None => session
                        .document()
                        .children_of(parent)
                        .ok_or("unknown parent")?
                        .len(),
                };
                Edit::InsertLayer {
                    parent,
                    index,
                    layer: Layer {
                        transform: slopshop_core::Affine::IDENTITY,
                        clipped: false,
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
            EditRequest::SetFillColor { id, color } => {
                let [r, g, b, a] = color;
                Edit::SetFillColor {
                    id: LayerId::from_raw(id),
                    color: LinearRgba::from_srgb_encoded_to_working(r, g, b, a),
                }
            }
            EditRequest::RemoveLayer { id } => Edit::RemoveLayer {
                id: LayerId::from_raw(id),
            },
            EditRequest::AddEmptyLayer {
                name,
                parent,
                index,
            } => {
                let size = session.document().size();
                // One shared transparent tile, whatever the canvas size.
                let image = slopshop_core::RasterImage::from_placed(
                    size,
                    slopshop_core::color::PixelFormat::RGBA8_SRGB,
                    slopshop_core::Rect::new(0, 0, 0, 0),
                    &[],
                    &[0, 0, 0, 0],
                )
                .map_err(|e| e.to_string())?;
                Edit::InsertLayer {
                    parent: parent.map(LayerId::from_raw),
                    index,
                    layer: Layer {
                        transform: slopshop_core::Affine::IDENTITY,
                        clipped: false,
                        id: session.allocate_layer_id(),
                        name,
                        visible: true,
                        opacity: 1.0,
                        blend_mode: BlendMode::Normal,
                        mask: None,
                        content: LayerContent::raster(std::sync::Arc::new(image)),
                    },
                }
            }
            EditRequest::ApplyEffect {
                ids,
                adjustment,
                values,
                curves,
                gradient,
            } => {
                let mut built = Adjustment::from_params(&adjustment, &values).ok_or(format!(
                    "unknown adjustment {adjustment} or too many values"
                ))?;
                if built.curves().is_some() {
                    built = curves_adjustment(curves.as_deref())?;
                }
                built = with_gradient(built, gradient.as_deref())?;
                let ids: Vec<LayerId> = ids.into_iter().map(LayerId::from_raw).collect();
                Edit::apply_effect(session.document(), &ids, built).map_err(|e| e.to_string())?
            }
            EditRequest::AutoLevels { ids, correction } => {
                use slopshop_core::auto::{AutoKind, auto_levels};
                let kind = match correction.as_str() {
                    "tone" => AutoKind::Tone,
                    "contrast" => AutoKind::Contrast,
                    "color" => AutoKind::Color,
                    other => return Err(format!("unknown automatic correction {other}")),
                };
                let ids: Vec<LayerId> = ids.into_iter().map(LayerId::from_raw).collect();
                match auto_levels(session.document(), kind).map_err(|e| e.to_string())? {
                    Some(levels) => Edit::apply_effect(session.document(), &ids, levels)
                        .map_err(|e| e.to_string())?,
                    // An empty batch leaves no undo entry.
                    None => Edit::Batch(Vec::new()),
                }
            }
            EditRequest::PreviewEffect { ids, adjustment } => {
                let adjustment = Adjustment::defaults(&adjustment)
                    .ok_or(format!("unknown adjustment {adjustment}"))?;
                let ids: Vec<LayerId> = ids.into_iter().map(LayerId::from_raw).collect();
                let doc = session.document();
                let mask = doc.selection().map(|s| slopshop_core::LayerMask {
                    image: std::sync::Arc::clone(s.image()),
                    enabled: true,
                    replaces_alpha: false,
                    original: None,
                });
                // Each above its layer, the highest first so that the places stay right.
                let mut places: Vec<(Option<LayerId>, usize)> = Edit::effect_targets(doc, &ids)
                    .into_iter()
                    .filter_map(|id| doc.locate(id))
                    .collect();
                places.sort_by_key(|p| std::cmp::Reverse(p.1));
                let mut edits = Vec::with_capacity(places.len());
                for (parent, index) in places {
                    edits.push(Edit::InsertLayer {
                        parent,
                        index: index + 1,
                        layer: Layer {
                            transform: slopshop_core::Affine::IDENTITY,
                            clipped: true,
                            id: session.allocate_layer_id(),
                            name: adjustment.id().to_owned(),
                            visible: true,
                            opacity: 1.0,
                            blend_mode: BlendMode::Normal,
                            mask: mask.clone(),
                            content: LayerContent::Adjustment { adjustment },
                        },
                    });
                }
                Edit::Batch(edits)
            }
            EditRequest::DeleteStackEntry { id, index } => {
                Edit::delete_entry(session.document(), LayerId::from_raw(id), index)
                    .map_err(|e| e.to_string())?
            }
            EditRequest::DeletePaint { ids } => {
                let ids: Vec<LayerId> = ids.into_iter().map(LayerId::from_raw).collect();
                Edit::delete_paint(session.document(), &ids).map_err(|e| e.to_string())?
            }
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
            EditRequest::MoveLayer { id, parent, index } => Edit::MoveLayer {
                id: LayerId::from_raw(id),
                parent: parent.map(LayerId::from_raw),
                index,
            },
            EditRequest::MoveLayers { ids, parent, index } => {
                let ids: Vec<LayerId> = ids.into_iter().map(LayerId::from_raw).collect();
                let parent = parent.map(LayerId::from_raw);
                Edit::move_layers(session.document(), &ids, parent, index)
                    .map_err(|e| e.to_string())?
            }
            EditRequest::AddGroup {
                name,
                parent,
                index,
            } => Edit::InsertLayer {
                parent: parent.map(LayerId::from_raw),
                index,
                layer: new_group(session, name),
            },
            EditRequest::AddAdjustmentLayer {
                name,
                adjustment,
                parent,
                index,
            } => {
                let adjustment = Adjustment::defaults(&adjustment)
                    .ok_or(format!("unknown adjustment {adjustment}"))?;
                Edit::InsertLayer {
                    parent: parent.map(LayerId::from_raw),
                    index,
                    layer: Layer {
                        transform: slopshop_core::Affine::IDENTITY,
                        clipped: false,
                        id: session.allocate_layer_id(),
                        name,
                        visible: true,
                        opacity: 1.0,
                        blend_mode: BlendMode::Normal,
                        mask: None,
                        content: LayerContent::Adjustment { adjustment },
                    },
                }
            }
            EditRequest::SetAdjustment {
                id,
                adjustment,
                values,
                curves,
                gradient,
            } => {
                let mut built = Adjustment::from_params(&adjustment, &values).ok_or(format!(
                    "unknown adjustment {adjustment} or too many values"
                ))?;
                if built.curves().is_some() {
                    built = curves_adjustment(curves.as_deref())?;
                }
                built = with_gradient(built, gradient.as_deref())?;
                Edit::SetAdjustment {
                    id: LayerId::from_raw(id),
                    adjustment: built,
                }
            }
            EditRequest::GroupLayers { ids, name } => {
                let ids: Vec<LayerId> = ids.into_iter().map(LayerId::from_raw).collect();
                let group = new_group(session, name);
                Edit::group_layers(session.document(), group, &ids).map_err(|e| e.to_string())?
            }
            EditRequest::Ungroup { ids } => {
                let ids: Vec<LayerId> = ids.into_iter().map(LayerId::from_raw).collect();
                Edit::ungroup_layers(session.document(), &ids).map_err(|e| e.to_string())?
            }
            EditRequest::ArrangeLayers { ids, arrange } => {
                let arrange = match arrange.as_str() {
                    "front" => Arrange::Front,
                    "forward" => Arrange::Forward,
                    "backward" => Arrange::Backward,
                    "back" => Arrange::Back,
                    other => return Err(format!("unknown arrangement {other}")),
                };
                let ids: Vec<LayerId> = ids.into_iter().map(LayerId::from_raw).collect();
                Edit::arrange_layers(session.document(), &ids, arrange)
                    .map_err(|e| e.to_string())?
            }
            EditRequest::DuplicateLayers { ids, name_format } => {
                let ids: Vec<LayerId> = ids.into_iter().map(LayerId::from_raw).collect();
                session
                    .duplicate_layers_edit(&ids, |name| name_format.replace("{name}", name))
                    .map_err(|e| e.to_string())?
            }
            EditRequest::DuplicateTransformLayers {
                ids,
                name_format,
                matrix,
            } => {
                let ids: Vec<LayerId> = ids.into_iter().map(LayerId::from_raw).collect();
                let duplicate = session
                    .duplicate_layers_edit(&ids, |name| name_format.replace("{name}", name))
                    .map_err(|e| e.to_string())?;
                // The copies' ids, and their places once inserted, from a plan of the document.
                let Edit::Batch(insertions) = &duplicate else {
                    return Err("duplicating gave no copies".to_owned());
                };
                let copies: Vec<LayerId> = insertions
                    .iter()
                    .filter_map(|edit| match edit {
                        Edit::InsertLayer { layer, .. } => Some(layer.id),
                        _ => None,
                    })
                    .collect();
                let mut plan = session.document().clone();
                duplicate
                    .clone()
                    .apply(&mut plan)
                    .map_err(|e| e.to_string())?;
                let transform = Edit::transform_layers(
                    &plan,
                    &copies,
                    slopshop_core::Affine::from_array(matrix),
                )
                .map_err(|e| e.to_string())?;
                Edit::Batch(vec![duplicate, transform])
            }
            EditRequest::TranslateLayers { ids, dx, dy } => {
                let ids: Vec<LayerId> = ids.into_iter().map(LayerId::from_raw).collect();
                Edit::translate_layers(session.document(), &ids, dx, dy)
                    .map_err(|e| e.to_string())?
            }
            EditRequest::TransformLayers { ids, matrix } => {
                let ids: Vec<LayerId> = ids.into_iter().map(LayerId::from_raw).collect();
                Edit::transform_layers(
                    session.document(),
                    &ids,
                    slopshop_core::Affine::from_array(matrix),
                )
                .map_err(|e| e.to_string())?
            }
            EditRequest::ResizeImage {
                width,
                height,
                resolution,
            } => {
                let doc = session.document();
                let size = Size::new(width, height);
                let resize = Edit::resize_image(doc, size).map_err(|e| e.to_string())?;
                // Resample off (Photoshop): only the resolution changes.
                match resolution.filter(|&ppi| ppi != doc.resolution()) {
                    Some(ppi) if size == doc.size() => Edit::SetResolution { ppi },
                    Some(ppi) => Edit::Batch(vec![resize, Edit::SetResolution { ppi }]),
                    None => resize,
                }
            }
            EditRequest::CanvasSize {
                width,
                height,
                anchor: [x, y],
            } => Edit::canvas_size(session.document(), Size::new(width, height), (x, y))
                .map_err(|e| e.to_string())?,
            EditRequest::Crop {
                x,
                y,
                width,
                height,
            } => {
                Edit::crop(session.document(), [x, y, width, height]).map_err(|e| e.to_string())?
            }
            EditRequest::RotateImage { turn } => {
                let turn = match turn.as_str() {
                    "clockwise" => ImageTurn::Clockwise,
                    "counterClockwise" => ImageTurn::CounterClockwise,
                    "halfTurn" => ImageTurn::HalfTurn,
                    "flipHorizontal" => ImageTurn::FlipHorizontal,
                    "flipVertical" => ImageTurn::FlipVertical,
                    other => return Err(format!("unknown image turn {other}")),
                };
                Edit::rotate_image(session.document(), turn).map_err(|e| e.to_string())?
            }
            EditRequest::RotateImageBy { degrees } => {
                Edit::rotate_image_by(session.document(), degrees).map_err(|e| e.to_string())?
            }
            // Nothing to do: an empty batch, which leaves no undo entry.
            EditRequest::RevealAll => {
                Edit::reveal_all(session.document(), crate::NEW_DOCUMENT_MAX_SIDE)
                    .map_err(|e| e.to_string())?
                    .unwrap_or(Edit::Batch(Vec::new()))
            }
            EditRequest::Trim {
                basis,
                top,
                bottom,
                left,
                right,
            } => {
                use slopshop_core::trim::{TrimBasis, TrimSides, trim_area};
                let basis = match basis.as_str() {
                    "transparent" => TrimBasis::Transparent,
                    "topLeft" => TrimBasis::TopLeftColor,
                    "bottomRight" => TrimBasis::BottomRightColor,
                    other => return Err(format!("unknown trim basis {other}")),
                };
                let sides = TrimSides {
                    top,
                    bottom,
                    left,
                    right,
                };
                match trim_area(session.document(), basis, sides).map_err(|e| e.to_string())? {
                    Some(area) => {
                        Edit::crop(session.document(), area).map_err(|e| e.to_string())?
                    }
                    None => Edit::Batch(Vec::new()),
                }
            }
            EditRequest::SetLayerClipped { id, clipped } => Edit::SetLayerClipped {
                id: LayerId::from_raw(id),
                clipped,
            },
            EditRequest::SetGroupPassThrough { id, pass_through } => Edit::SetGroupPassThrough {
                id: LayerId::from_raw(id),
                pass_through,
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

/// A new empty group, isolated (blend mode Normal), with a fresh id: an adjustment inside
/// changes the group only. Photoshop passes through by default; the maintainer chose isolation,
/// Pass Through stays one choice away in the blend modes.
fn new_group(session: &mut Session, name: String) -> Layer {
    Layer {
        transform: slopshop_core::Affine::IDENTITY,
        clipped: false,
        id: session.allocate_layer_id(),
        name,
        visible: true,
        opacity: 1.0,
        blend_mode: BlendMode::Normal,
        mask: None,
        content: LayerContent::Group {
            children: Vec::new(),
            pass_through: false,
        },
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
    /// False when part of the view was shown from a coarser level of the display cache while
    /// its tiles are composited (ADR 0022): present again to refine it.
    pub complete: bool,
    pub revision: u64,
    /// 1.0 = 100%.
    pub zoom: f64,
    /// The view presented: its document point at the top left of the canvas.
    pub origin: [f64; 2],
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
/// | 12     | u32  | flags (bit 0: view is in fit mode; bit 1:     |
/// |        |      | incomplete, render it again)                  |
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
    /// Shown coarser or approximated meanwhile (a layer's stack evaluated): render it again.
    pub incomplete: bool,
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
        let flags = u32::from(self.fit) | (u32::from(self.incomplete) << 1);
        bytes[12..16].copy_from_slice(&flags.to_le_bytes());
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

/// A Curves adjustment from four lists of points (composite, red, green, blue).
fn curves_adjustment(lists: Option<&[Vec<[u8; 2]>]>) -> Result<Adjustment, String> {
    let lists = lists
        .filter(|lists| lists.len() == 4)
        .ok_or("curves need four lists of points")?;
    let curve =
        |points: &[[u8; 2]]| Curve::new(points).ok_or(format!("invalid curve points {points:?}"));
    Ok(Adjustment::Curves {
        rgb: curve(&lists[0])?,
        red: curve(&lists[1])?,
        green: curve(&lists[2])?,
        blue: curve(&lists[3])?,
    })
}

/// Gradient Map with the stops `[location, r, g, b]` given (its reverse flag kept); other
/// adjustments as they are.
fn with_gradient(adjustment: Adjustment, stops: Option<&[[u16; 4]]>) -> Result<Adjustment, String> {
    let Adjustment::GradientMap { reverse, .. } = adjustment else {
        return Ok(adjustment);
    };
    use slopshop_core::gradient::{Gradient, GradientStop};
    let stops = stops.ok_or("a gradient map needs its stops")?;
    let stops: Vec<GradientStop> = stops
        .iter()
        .map(|&[location, r, g, b]| {
            let byte = |v: u16| u8::try_from(v).map_err(|_| format!("invalid color value {v}"));
            Ok(GradientStop {
                location,
                color: [byte(r)?, byte(g)?, byte(b)?],
            })
        })
        .collect::<Result<_, String>>()?;
    let gradient = Gradient::new(&stops).ok_or(format!("invalid gradient stops {stops:?}"))?;
    Ok(Adjustment::GradientMap { gradient, reverse })
}

/// A gradient's stops as the UI gets them, `[location, r, g, b]`.
fn gradient_stops(gradient: &slopshop_core::gradient::Gradient) -> Vec<[u16; 4]> {
    gradient
        .stops()
        .iter()
        .map(|s| {
            let [r, g, b] = s.color.map(u16::from);
            [s.location, r, g, b]
        })
        .collect()
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
    Psd,
    Psb,
    Bmp,
    Tga,
    Pnm,
    Pfm,
    Avif,
    Jxl,
    Qoi,
    #[serde(rename = "ff")]
    Farbfeld,
    Hdr,
    Ico,
    Gif,
    Dds,
    Fits,
    #[serde(rename = "dcm")]
    Dicom,
    Pdf,
    #[serde(rename = "jp2")]
    Jpeg2000,
}

impl ExportFormatId {
    pub fn kind(self) -> ExportFormatKind {
        match self {
            ExportFormatId::Png => ExportFormatKind::Png,
            ExportFormatId::Tiff => ExportFormatKind::Tiff,
            ExportFormatId::Exr => ExportFormatKind::Exr,
            ExportFormatId::Jpeg => ExportFormatKind::Jpeg,
            ExportFormatId::Webp => ExportFormatKind::Webp,
            ExportFormatId::Psd => ExportFormatKind::Psd,
            ExportFormatId::Psb => ExportFormatKind::Psb,
            ExportFormatId::Bmp => ExportFormatKind::Bmp,
            ExportFormatId::Tga => ExportFormatKind::Tga,
            ExportFormatId::Pnm => ExportFormatKind::Pnm,
            ExportFormatId::Pfm => ExportFormatKind::Pfm,
            ExportFormatId::Avif => ExportFormatKind::Avif,
            ExportFormatId::Jxl => ExportFormatKind::Jxl,
            ExportFormatId::Qoi => ExportFormatKind::Qoi,
            ExportFormatId::Farbfeld => ExportFormatKind::Farbfeld,
            ExportFormatId::Hdr => ExportFormatKind::Hdr,
            ExportFormatId::Ico => ExportFormatKind::Ico,
            ExportFormatId::Gif => ExportFormatKind::Gif,
            ExportFormatId::Dds => ExportFormatKind::Dds,
            ExportFormatId::Fits => ExportFormatKind::Fits,
            ExportFormatId::Dicom => ExportFormatKind::Dicom,
            ExportFormatId::Pdf => ExportFormatKind::Pdf,
            ExportFormatId::Jpeg2000 => ExportFormatKind::Jpeg2000,
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
    Rle,
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
            ExportFormat::Psd { depth } | ExportFormat::Psb { depth } => (
                if spec.format.kind() == ExportFormatKind::Psb {
                    ExportFormatId::Psb
                } else {
                    ExportFormatId::Psd
                },
                match depth {
                    PsdDepth::U8 => S::U8,
                    PsdDepth::U16 => S::U16,
                },
                None,
            ),
            ExportFormat::Bmp => (ExportFormatId::Bmp, S::U8, None),
            ExportFormat::Pnm { depth } => (
                ExportFormatId::Pnm,
                match depth {
                    PngDepth::U8 => S::U8,
                    PngDepth::U16 => S::U16,
                },
                None,
            ),
            ExportFormat::Pfm => (ExportFormatId::Pfm, S::F32, None),
            ExportFormat::Qoi => (ExportFormatId::Qoi, S::U8, None),
            ExportFormat::Farbfeld => (ExportFormatId::Farbfeld, S::U16, None),
            ExportFormat::Hdr => (ExportFormatId::Hdr, S::F32, None),
            ExportFormat::Ico => (ExportFormatId::Ico, S::U8, None),
            ExportFormat::Gif => (ExportFormatId::Gif, S::U8, None),
            ExportFormat::Dds => (ExportFormatId::Dds, S::U8, None),
            ExportFormat::Pdf => (ExportFormatId::Pdf, S::U8, None),
            ExportFormat::Jpeg2000 { depth, compression } => {
                let compression = match compression {
                    Jpeg2000Compression::Lossy { quality: q } => {
                        quality = Some(q);
                        C::Lossy
                    }
                    Jpeg2000Compression::Lossless => C::Lossless,
                };
                let sample = match depth {
                    PngDepth::U8 => S::U8,
                    PngDepth::U16 => S::U16,
                };
                (ExportFormatId::Jpeg2000, sample, Some(compression))
            }
            ExportFormat::Fits { sample } => (
                ExportFormatId::Fits,
                match sample {
                    TiffSample::U8 => S::U8,
                    TiffSample::U16 => S::U16,
                    TiffSample::F32 => S::F32,
                },
                None,
            ),
            ExportFormat::Dicom { depth } => (
                ExportFormatId::Dicom,
                match depth {
                    PngDepth::U8 => S::U8,
                    PngDepth::U16 => S::U16,
                },
                None,
            ),
            ExportFormat::Jxl { depth } => (
                ExportFormatId::Jxl,
                match depth {
                    PngDepth::U8 => S::U8,
                    PngDepth::U16 => S::U16,
                },
                None,
            ),
            ExportFormat::Avif { depth, quality: q } => {
                quality = Some(q);
                let sample = match depth {
                    AvifDepth::U8 => S::U8,
                    AvifDepth::U10 => S::U16,
                };
                (ExportFormatId::Avif, sample, None)
            }
            ExportFormat::Tga { compression } => (
                ExportFormatId::Tga,
                S::U8,
                Some(match compression {
                    TgaCompression::None => C::None,
                    TgaCompression::Rle => C::Rle,
                }),
            ),
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
        // Lossy WebP and lossy JPEG 2000 have a quality, their lossless modes none.
        let lossy = matches!(self.format, ExportFormatId::Webp | ExportFormatId::Jpeg2000)
            && self.compression == Some(C::Lossy);
        let quality_format = matches!(self.format, ExportFormatId::Jpeg | ExportFormatId::Avif);
        if !quality_format && !lossy && self.quality.is_some() {
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
            ExportFormatId::Psd | ExportFormatId::Psb => {
                if self.compression.is_some() {
                    return Err(compression());
                }
                // The layers keep their transparency.
                if !self.keep_alpha {
                    return Err(invalid("dropping alpha".to_owned()));
                }
                let depth = match self.sample {
                    S::U8 => PsdDepth::U8,
                    S::U16 => PsdDepth::U16,
                    S::F16 | S::F32 => return Err(sample()),
                };
                if self.format == ExportFormatId::Psb {
                    ExportFormat::Psb { depth }
                } else {
                    ExportFormat::Psd { depth }
                }
            }
            ExportFormatId::Pnm => {
                if self.compression.is_some() {
                    return Err(compression());
                }
                ExportFormat::Pnm {
                    depth: match self.sample {
                        S::U8 => PngDepth::U8,
                        S::U16 => PngDepth::U16,
                        S::F16 | S::F32 => return Err(sample()),
                    },
                }
            }
            ExportFormatId::Jxl => {
                if self.compression.is_some() {
                    return Err(compression());
                }
                ExportFormat::Jxl {
                    depth: match self.sample {
                        S::U8 => PngDepth::U8,
                        S::U16 => PngDepth::U16,
                        S::F16 | S::F32 => return Err(sample()),
                    },
                }
            }
            ExportFormatId::Jpeg2000 => ExportFormat::Jpeg2000 {
                depth: match self.sample {
                    S::U8 => PngDepth::U8,
                    S::U16 => PngDepth::U16,
                    S::F16 | S::F32 => return Err(sample()),
                },
                compression: match self.compression {
                    Some(C::Lossless) => Jpeg2000Compression::Lossless,
                    Some(C::Lossy) => Jpeg2000Compression::Lossy {
                        quality: self
                            .quality
                            .filter(|q| (1..=100).contains(q))
                            .ok_or_else(quality)?,
                    },
                    _ => return Err(compression()),
                },
            },
            ExportFormatId::Fits => {
                if self.compression.is_some() {
                    return Err(compression());
                }
                ExportFormat::Fits {
                    sample: match self.sample {
                        S::U8 => TiffSample::U8,
                        S::U16 => TiffSample::U16,
                        S::F32 => TiffSample::F32,
                        S::F16 => return Err(sample()),
                    },
                }
            }
            ExportFormatId::Dicom => {
                if self.compression.is_some() {
                    return Err(compression());
                }
                ExportFormat::Dicom {
                    depth: match self.sample {
                        S::U8 => PngDepth::U8,
                        S::U16 => PngDepth::U16,
                        S::F16 | S::F32 => return Err(sample()),
                    },
                }
            }
            ExportFormatId::Avif => {
                if self.compression.is_some() {
                    return Err(compression());
                }
                ExportFormat::Avif {
                    depth: match self.sample {
                        S::U8 => AvifDepth::U8,
                        S::U16 => AvifDepth::U10,
                        S::F16 | S::F32 => return Err(sample()),
                    },
                    quality: self.quality.filter(|q| *q <= 100).ok_or_else(quality)?,
                }
            }
            ExportFormatId::Pfm
            | ExportFormatId::Qoi
            | ExportFormatId::Farbfeld
            | ExportFormatId::Hdr
            | ExportFormatId::Ico
            | ExportFormatId::Gif
            | ExportFormatId::Dds
            | ExportFormatId::Pdf => {
                if self.compression.is_some() {
                    return Err(compression());
                }
                // Formats with one sample type and no settings.
                let (format, only) = match self.format {
                    ExportFormatId::Qoi => (ExportFormat::Qoi, S::U8),
                    ExportFormatId::Farbfeld => (ExportFormat::Farbfeld, S::U16),
                    ExportFormatId::Hdr => (ExportFormat::Hdr, S::F32),
                    ExportFormatId::Ico => (ExportFormat::Ico, S::U8),
                    ExportFormatId::Gif => (ExportFormat::Gif, S::U8),
                    ExportFormatId::Dds => (ExportFormat::Dds, S::U8),
                    ExportFormatId::Pdf => (ExportFormat::Pdf, S::U8),
                    ExportFormatId::Pfm => (ExportFormat::Pfm, S::F32),
                    other => return Err(invalid(format!("{other:?} settings"))),
                };
                if self.sample != only {
                    return Err(sample());
                }
                format
            }
            ExportFormatId::Bmp | ExportFormatId::Tga => {
                if self.sample != S::U8 {
                    return Err(sample());
                }
                match (self.format, self.compression) {
                    (ExportFormatId::Bmp, None) => ExportFormat::Bmp,
                    (ExportFormatId::Tga, Some(C::None)) => ExportFormat::Tga {
                        compression: TgaCompression::None,
                    },
                    (ExportFormatId::Tga, Some(C::Rle)) => ExportFormat::Tga {
                        compression: TgaCompression::Rle,
                    },
                    _ => return Err(compression()),
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
            // The document's, set by the caller: the UI does not choose them.
            blend_space: BlendSpace::default(),
            resolution: None,
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
                resolution: None,
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
        // The resolution is the document's, set by the caller, not the UI's.
        let unplaced = |kind| ExportSpec {
            resolution: None,
            ..default_spec(kind, &document)
        };
        for format in [
            ExportFormatId::Png,
            ExportFormatId::Tiff,
            ExportFormatId::Exr,
            ExportFormatId::Jpeg,
            ExportFormatId::Webp,
            ExportFormatId::Psd,
            ExportFormatId::Psb,
            ExportFormatId::Bmp,
            ExportFormatId::Tga,
            ExportFormatId::Pnm,
            ExportFormatId::Pfm,
            ExportFormatId::Avif,
            ExportFormatId::Jxl,
            ExportFormatId::Qoi,
            ExportFormatId::Farbfeld,
            ExportFormatId::Hdr,
            ExportFormatId::Ico,
            ExportFormatId::Gif,
            ExportFormatId::Dds,
            ExportFormatId::Fits,
            ExportFormatId::Dicom,
            ExportFormatId::Pdf,
            ExportFormatId::Jpeg2000,
        ] {
            let spec = unplaced(format.kind());
            let dto = ExportSpecDto::new(&spec);
            assert_eq!(dto.format, format);
            assert_eq!(dto.to_spec(None).unwrap(), spec, "{format:?}");
        }
        // As the CLI and the UI spell it.
        assert_eq!(
            serde_json::to_string(&ExportFormatId::Farbfeld).unwrap(),
            r#""ff""#
        );
        assert_eq!(
            serde_json::to_string(&ExportFormatId::Dicom).unwrap(),
            r#""dcm""#
        );
        assert_eq!(
            serde_json::to_string(&ExportFormatId::Jpeg2000).unwrap(),
            r#""jp2""#
        );
        // Lossy JPEG 2000 keeps its quality through the DTO.
        let lossy = ExportSpec {
            format: ExportFormat::Jpeg2000 {
                depth: PngDepth::U16,
                compression: Jpeg2000Compression::Lossy { quality: 42 },
            },
            ..unplaced(ExportFormatKind::Jpeg2000)
        };
        let dto = ExportSpecDto::new(&lossy);
        assert_eq!(dto.quality, Some(42));
        assert_eq!(dto.to_spec(None).unwrap(), lossy);
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
            resolution: None,
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
                r#"{"format":"svg","sample":"u8","compression":null,"quality":null,"subsampling":null,"space":"srgb","keepAlpha":false,"matte":[1.0,1.0,1.0],"dither":false,"gray":false}"#
            )
            .is_err()
        );
        assert_eq!(
            with(&|d| {
                d.format = ExportFormatId::Gif;
                d.sample = S::U16;
                d.compression = None;
            }),
            invalid,
            "16-bit GIF"
        );
        assert_eq!(
            with(&|d| {
                d.format = ExportFormatId::Gif;
                d.sample = S::U8;
                d.compression = None;
            }),
            Ok(()),
            "GIF"
        );
        assert_eq!(
            with(&|d| {
                d.format = ExportFormatId::Hdr;
                d.sample = S::F32;
                d.compression = None;
                d.keep_alpha = false;
            }),
            Err("unsupportedSpace"),
            "Radiance HDR is linear only"
        );
        let fits = |sample| {
            move |d: &mut ExportSpecDto| {
                d.format = ExportFormatId::Fits;
                d.sample = sample;
                d.compression = None;
                d.keep_alpha = false;
                d.gray = true;
            }
        };
        assert_eq!(with(&fits(S::F32)), Ok(()), "float FITS");
        assert_eq!(with(&fits(S::F16)), invalid, "half-float FITS");
        assert_eq!(
            with(&|d| {
                d.format = ExportFormatId::Dicom;
                d.sample = S::F32;
                d.compression = None;
                d.keep_alpha = false;
            }),
            invalid,
            "float DICOM"
        );
        assert_eq!(
            with(&|d| {
                d.format = ExportFormatId::Pdf;
                d.sample = S::U8;
                d.compression = None;
                d.gray = true;
                d.space = "display-p3".to_owned();
            }),
            Ok(()),
            "gray PDF"
        );
        assert_eq!(
            with(&|d| {
                d.format = ExportFormatId::Pdf;
                d.sample = S::U16;
                d.compression = None;
            }),
            invalid,
            "16-bit PDF"
        );
        let jp2 = |compression, quality| {
            move |d: &mut ExportSpecDto| {
                d.format = ExportFormatId::Jpeg2000;
                d.sample = S::U16;
                d.compression = Some(compression);
                d.quality = quality;
            }
        };
        assert_eq!(with(&jp2(C::Lossless, None)), Ok(()), "lossless JPEG 2000");
        assert_eq!(with(&jp2(C::Lossy, Some(50))), Ok(()), "lossy JPEG 2000");
        assert_eq!(with(&jp2(C::Lossy, None)), invalid, "no quality");
        assert_eq!(with(&jp2(C::Lossy, Some(0))), invalid, "quality 0");
        assert_eq!(
            with(&jp2(C::Lossless, Some(50))),
            invalid,
            "lossless quality"
        );
        assert_eq!(with(&jp2(C::Rle, None)), invalid, "RLE JPEG 2000");
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
        let psd = |change: &dyn Fn(&mut ExportSpecDto)| {
            with(&|d| {
                d.format = ExportFormatId::Psd;
                d.compression = None;
                change(d);
            })
        };
        assert_eq!(psd(&|_| {}), Ok(()), "8-bit PSD");
        assert_eq!(psd(&|d| d.sample = S::U16), Ok(()), "16-bit PSD");
        assert_eq!(psd(&|d| d.sample = S::F32), invalid, "float PSD");
        assert_eq!(psd(&|d| d.compression = Some(C::Fast)), invalid);
        assert_eq!(psd(&|d| d.keep_alpha = false), invalid, "PSD without alpha");
        assert_eq!(psd(&|d| d.gray = true), invalid, "gray PSD");
        let avif = |change: &dyn Fn(&mut ExportSpecDto)| {
            with(&|d| {
                d.format = ExportFormatId::Avif;
                d.compression = None;
                d.quality = Some(80);
                change(d);
            })
        };
        assert_eq!(avif(&|_| {}), Ok(()), "8-bit AVIF");
        assert_eq!(avif(&|d| d.sample = S::U16), Ok(()), "10-bit AVIF");
        assert_eq!(
            avif(&|d| d.quality = None),
            invalid,
            "AVIF without a quality"
        );
        assert_eq!(avif(&|d| d.quality = Some(101)), invalid, "quality 101");
        assert_eq!(avif(&|d| d.sample = S::F32), invalid, "float AVIF");
        assert_eq!(avif(&|d| d.compression = Some(C::Lossless)), invalid);
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
            resolution: None,
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
