//! Photoshop layers, stage 2 of the formats plan (docs/formats.md): the layer records and their
//! channels become a document. Each layer is a raster the size of the canvas whose area outside
//! the layer's bounds is one shared tile ([`RasterImage::from_placed`]), so small layers on a
//! large canvas cost what their pixels cost.
//!
//! Read from the layer and mask information section: the layer info (in the section itself, or
//! for 16/32-bit documents in its `Lr16`/`Lr32` tagged block), one record per layer (bounds,
//! channels, blend mode, opacity, flags, mask, name, tagged blocks), then every layer's channels,
//! each compressed on its own like the composite. Layers come bottom to top, as in a document.
//!
//! What the engine holds: pixel layers, solid color fill layers, groups (ADR 0015: pass-through
//! or isolated, nested) and clipping masks (ADR 0016) with their name, visibility, opacity (times their fill
//! opacity), blend mode and layer mask, at the document's depth and in its color space; 8/16-bit
//! documents blend in perceptual space and 32-bit ones in linear light, as in Photoshop. What it
//! does not hold yet is approximated and reported, layer by layer:
//! - layer styles and advanced blending ("Blend If", the fill opacity of the
//!   modes where it differs from opacity): ignored;
//! - adjustment layers, and gradient, pattern or vector-only fill layers without pixels: left
//!   out;
//! - text, shapes, smart objects, fill layers and vector masks: their pixels, which Photoshop
//!   stores (a vector mask as the user mask it renders);
//! - mask density and feather, and a vector mask beside a pixel mask: ignored;
//! - pixels outside the canvas: cropped.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;

use flate2::read::ZlibDecoder;
use slopshop_core::adjust::Adjustment;
use slopshop_core::color::{
    AlphaMode, ChannelLayout, ColorSpace, PixelFormat, SampleType, WORKING_SPACE,
};
use slopshop_core::curve::Curve;
use slopshop_core::{
    BlendMode, BlendSpace, Document, Layer, LayerContent, LayerId, LayerMask, LinearRgba,
    RasterImage, Rect, Size,
};

use super::{Header, Input, Mode, Samples, corrupt, unpack_bits, unpredict};
use crate::{ImportError, ImportWarning, ImportedLayers, MAX_IMPORT_BYTES, resolve_color_space};

/// Tagged blocks holding the layer info of 8-bit (rarely), 16-bit and 32-bit documents.
const LAYER_INFO_KEYS: [&[u8; 4]; 3] = [b"Layr", b"Lr16", b"Lr32"];
/// Tagged blocks whose length is 64-bit in PSB files.
const LONG_KEYS: [&[u8; 4]; 13] = [
    b"LMsk", b"Lr16", b"Lr32", b"Layr", b"Mt16", b"Mt32", b"Mtrn", b"Alph", b"FMsk", b"lnk2",
    b"FEid", b"FXid", b"PxSD",
];
/// Layer styles (effects).
const STYLE_KEYS: [&[u8; 4]; 2] = [b"lfx2", b"lmfx"];
/// Layers whose pixels Photoshop renders from other data: text, smart objects, vector shapes
/// and masks.
const RASTERIZED_KEYS: [&[u8; 4]; 11] = [
    b"TySh", b"tySh", b"SoLd", b"SoLE", b"PlLd", b"plLd", b"vmsk", b"vsms", b"vogk", b"vscg",
    b"vstk",
];
/// Fill layers: imported as their pixels when they have some.
const FILL_KEYS: [&[u8; 4]; 3] = [b"SoCo", b"GdFl", b"PtFl"];
/// Adjustment layers: parameters, no pixels.
const ADJUSTMENT_KEYS: [&[u8; 4]; 17] = [
    b"brit", b"levl", b"curv", b"expA", b"vibA", b"hue ", b"hue2", b"blnc", b"blwh", b"phfl",
    b"mixr", b"clrL", b"nvrt", b"post", b"thrs", b"grdm", b"selc",
];
/// Photoshop layers have at most a few channels (colors, transparency, two masks).
const MAX_LAYER_CHANNELS: u16 = 64;
/// Channel ids: transparency, user mask, real user mask (with a vector mask too).
const TRANSPARENCY: i16 = -1;
const USER_MASK: i16 = -2;
const REAL_USER_MASK: i16 = -3;

/// Bounds in document coordinates, as stored: they may extend past the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Bounds {
    top: i64,
    left: i64,
    bottom: i64,
    right: i64,
}

impl Bounds {
    /// Top, left, bottom, right as big-endian `i32`s.
    fn parse(b: &[u8]) -> Self {
        let at = |i: usize| i64::from(i32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]));
        Self {
            top: at(0),
            left: at(4),
            bottom: at(8),
            right: at(12),
        }
    }

    fn check(self, max_side: u32) -> Result<Self, ImportError> {
        let max = i64::from(max_side);
        if self.bottom < self.top
            || self.right < self.left
            || self.bottom - self.top > max
            || self.right - self.left > max
        {
            return Err(corrupt("invalid layer bounds"));
        }
        Ok(self)
    }

    fn width(self) -> usize {
        (self.right - self.left) as usize
    }

    fn height(self) -> usize {
        (self.bottom - self.top) as usize
    }

    /// The part inside a canvas of `size` (empty when none).
    fn visible(self, size: Size) -> Rect {
        let left = self.left.clamp(0, i64::from(size.width));
        let right = self.right.clamp(left, i64::from(size.width));
        let top = self.top.clamp(0, i64::from(size.height));
        let bottom = self.bottom.clamp(top, i64::from(size.height));
        // Clamped to the canvas: every value fits in u32.
        Rect::new(
            left as u32,
            top as u32,
            (right - left) as u32,
            (bottom - top) as u32,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Pixels,
    /// A solid color fill layer without pixels: a fill of `Record::solid_color`.
    SolidColor,
    /// The top of a group: its name, visibility, opacity and blend mode.
    GroupStart,
    /// The bottom of a group (a hidden divider).
    GroupEnd,
    /// An adjustment layer this version reproduces (ADR 0020): `Record::adjustment`.
    Adjustment,
    /// No pixels to import (other adjustment layers, fill layers without pixels).
    Skipped,
}

#[derive(Debug, Clone, Copy)]
struct Mask {
    bounds: Bounds,
    /// 0 or 255: the mask's value outside its bounds.
    default: u8,
    disabled: bool,
}

#[derive(Debug)]
struct Record {
    bounds: Bounds,
    /// Channel id and stored length (compression field included).
    channels: Vec<(i16, u64)>,
    blend_key: [u8; 4],
    opacity: u8,
    fill: u8,
    clipping: bool,
    hidden: bool,
    /// The channel holding the mask and its placement.
    mask: Option<(i16, Mask)>,
    name: String,
    kind: Kind,
    /// Effects, or blending ranges other than the default.
    styles: bool,
    /// Pixels rendered from other data (text, shapes, smart objects, fills, vector masks).
    rasterized: bool,
    /// Mask density or feather, or a vector mask beside the pixel mask, left out.
    masks_simplified: bool,
    /// The color of a solid color fill layer, encoded (0–1) in the document's space.
    solid_color: Option<[f32; 3]>,
    /// The adjustment of an adjustment layer this version reproduces.
    adjustment: Option<Adjustment>,
    /// Some of its settings are left out (see `read_adjustment`).
    adjustment_approximated: bool,
}

/// Read the layers of a document positioned at its layer and mask information section.
/// `None` when it has no pixel layers.
pub(super) fn read<R: Read + Seek>(
    input: &mut Input<R>,
    header: &Header,
) -> Result<Option<ImportedLayers>, ImportError> {
    let section = input.length()?;
    if section == 0 {
        return Ok(None);
    }
    let end = section_end(input, section)?;
    if seek_layer_info(input, end)? {
        read_layer_info(input, header)
    } else {
        Ok(None)
    }
}

/// The signed layer count of the layer and mask section at `input` (0 when it has none or it is
/// unreadable; negative: the composite's first extra channel is its transparency), the input
/// then at the section's end.
pub(super) fn layer_count<R: Read + Seek>(input: &mut Input<R>) -> Result<i16, ImportError> {
    let section = input.length()?;
    let end = section_end(input, section)?;
    let count = if section > 0 && matches!(seek_layer_info(input, end), Ok(true)) {
        input.bytes().map(i16::from_be_bytes).unwrap_or(0)
    } else {
        0
    };
    input.r.seek(SeekFrom::Start(end))?;
    Ok(count)
}

/// Where the section of `length` bytes starting at `input` ends.
fn section_end<R: Read + Seek>(input: &mut Input<R>, length: u64) -> Result<u64, ImportError> {
    input
        .r
        .stream_position()?
        .checked_add(length)
        .ok_or_else(|| corrupt("section too large"))
}

/// Position `input` (inside the layer and mask section ending at `end`, at the layer info's
/// length) at the layer count of its layer info: the section's own when it has layers, else a
/// tagged block's after the global mask info (16/32-bit documents keep their layers there).
/// `false` when there is none.
fn seek_layer_info<R: Read + Seek>(input: &mut Input<R>, end: u64) -> Result<bool, ImportError> {
    let info = input.length()?;
    let info_start = input.r.stream_position()?;
    let info_end = section_end(input, info)?;
    if info >= 2 && i16::from_be_bytes(input.bytes()?) != 0 {
        input.r.seek(SeekFrom::Start(info_start))?;
        return Ok(true);
    }
    input.r.seek(SeekFrom::Start(info_end))?;
    let global_mask = u64::from(input.u32()?);
    input.skip(global_mask)?;
    while let Some((key, len)) = next_block(input, end)? {
        let data_start = input.r.stream_position()?;
        if LAYER_INFO_KEYS.contains(&&key) {
            return Ok(true);
        }
        let next = data_start
            .checked_add(len)
            .ok_or_else(|| corrupt("tagged block too large"))?;
        input.r.seek(SeekFrom::Start(next))?;
    }
    Ok(false)
}

/// The next tagged block of the section ending at `end`: its key and data length, the input
/// positioned at its data. Writers pad blocks differently: up to 3 padding bytes are skipped.
fn next_block<R: Read + Seek>(
    input: &mut Input<R>,
    end: u64,
) -> Result<Option<([u8; 4], u64)>, ImportError> {
    let at = input.r.stream_position()?;
    for pad in 0..4 {
        if at + pad + 12 > end {
            return Ok(None);
        }
        input.r.seek(SeekFrom::Start(at + pad))?;
        let signature = input.bytes::<4>()?;
        if &signature == b"8BIM" || &signature == b"8B64" {
            let key = input.bytes::<4>()?;
            let len = if input.big && LONG_KEYS.contains(&&key) {
                input.length()?
            } else {
                u64::from(input.u32()?)
            };
            return Ok(Some((key, len)));
        }
    }
    Ok(None)
}

/// The layer info: the count, the records, then the channels of every layer.
fn read_layer_info<R: Read + Seek>(
    input: &mut Input<R>,
    header: &Header,
) -> Result<Option<ImportedLayers>, ImportError> {
    // Negative: the composite's first extra channel is its transparency (see the composite).
    let count = i16::from_be_bytes(input.bytes()?).unsigned_abs();
    if count == 0 {
        return Ok(None);
    }
    let max_side = Header::max_side(input.big);
    let mut records = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        records.push(read_record(input, max_side)?);
    }
    if !records
        .iter()
        .any(|r| matches!(r.kind, Kind::Pixels | Kind::SolidColor | Kind::Adjustment))
    {
        return Ok(None);
    }

    let canvas = Size::new(header.width, header.height);
    let mut warnings = Vec::new();
    let space = header.color_space(&mut warnings);
    if header.mode == Mode::Duotone {
        warnings.push(ImportWarning::ColorInfoUnsupported);
    }
    if records.iter().any(|r| r.kind == Kind::Skipped) {
        warnings.push(ImportWarning::AdjustmentLayersSkipped);
    }
    let sample = match header.depth {
        16 => SampleType::U16,
        32 => SampleType::F32,
        _ => SampleType::U8,
    };
    check_layers_budget(&records, canvas, header, sample)?;

    // Channels, layer after layer: a batch of layers is read, then decoded in parallel.
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let context = Context {
        canvas,
        depth: header.depth,
        colors: header.color_channels(),
        sample,
        space,
        fill_space: resolve_color_space(
            false,
            None,
            header.icc_profile.as_deref(),
            &mut Vec::new(),
        ),
        big: input.big,
    };
    let mut built: Vec<Option<Built>> = Vec::with_capacity(records.len());
    for batch in records.chunks(threads) {
        let mut raws = Vec::with_capacity(batch.len());
        for record in batch {
            let mut raw = Vec::new();
            for &(id, len) in &record.channels {
                let wanted = match record.kind {
                    Kind::Pixels => context.wants(record, id),
                    Kind::SolidColor | Kind::GroupStart | Kind::Adjustment => {
                        record.mask.is_some_and(|(mask, _)| mask == id)
                    }
                    _ => false,
                };
                if wanted {
                    let dims = channel_dims(record, id);
                    raw.push((id, read_channel(input, len, dims, &context)?));
                } else {
                    input.skip(len)?;
                }
            }
            raws.push(raw);
        }
        let results: Vec<Result<Option<Built>, ImportError>> = std::thread::scope(|scope| {
            let workers: Vec<_> = batch
                .iter()
                .zip(&raws)
                .map(|(record, raw)| {
                    let context = &context;
                    scope.spawn(move || match record.kind {
                        Kind::Pixels => build_layer(record, raw, context).map(Some),
                        Kind::SolidColor => build_solid(record, raw, context).map(Some),
                        Kind::GroupStart => build_group(record, raw, context).map(Some),
                        Kind::Adjustment => build_adjustment(record, raw, context).map(Some),
                        _ => Ok(None),
                    })
                })
                .collect();
            workers
                .into_iter()
                .map(|w| {
                    w.join()
                        .unwrap_or_else(|_| Err(corrupt("layer decoder panicked")))
                })
                .collect()
        });
        for result in results {
            built.push(result?);
        }
    }

    // The tree: records come bottom to top, a group as its divider (bottom), its layers, then
    // its own record (top). A divider opens a level; the group's record closes it.
    let mut levels: Vec<Vec<Layer>> = vec![Vec::new()];
    let mut notes_of: HashMap<LayerId, Vec<ImportWarning>> = HashMap::new();
    let mut next_id = 1;
    for (record, built) in records.iter().zip(built) {
        if record.kind == Kind::GroupEnd {
            levels.push(Vec::new());
            continue;
        }
        let Some(built) = built else { continue };
        let pass_through = record.kind == Kind::GroupStart && &record.blend_key == b"pass";
        let (blend_mode, known_mode) = match blend_mode(&record.blend_key) {
            Some(mode) => (mode, true),
            None => (BlendMode::Normal, pass_through),
        };
        let mut notes = Vec::new();
        let fill_differs = record.fill < 255 && fill_is_not_opacity(blend_mode);
        if record.styles || !known_mode || fill_differs {
            notes.push(ImportWarning::LayerStylesIgnored);
        }
        if record.rasterized {
            notes.push(ImportWarning::LayersRasterized);
        }
        if record.masks_simplified {
            notes.push(ImportWarning::MasksSimplified);
        }
        if built.cropped {
            notes.push(ImportWarning::PixelsOutsideCanvas);
        }
        // Adjustment layers blend in normal mode only for now.
        if record.kind == Kind::Adjustment
            && (record.adjustment_approximated || blend_mode != BlendMode::Normal)
        {
            notes.push(ImportWarning::AdjustmentsApproximated);
        }
        let content = match built.content {
            LayerContent::Group { .. } => LayerContent::Group {
                // A group record without its divider (damaged files) is an empty group.
                children: if levels.len() > 1 {
                    levels.pop().unwrap_or_default()
                } else {
                    Vec::new()
                },
                pass_through,
            },
            content => content,
        };
        let id = LayerId::from_raw(next_id);
        next_id += 1;
        let opacity = f32::from(record.opacity) / 255.0 * f32::from(record.fill) / 255.0;
        if !notes.is_empty() {
            notes_of.insert(id, notes);
        }
        let layer = Layer {
            transform: slopshop_core::Affine::IDENTITY,
            clipped: record.clipping,
            id,
            name: record.name.clone(),
            visible: !record.hidden,
            opacity: opacity.clamp(0.0, 1.0),
            blend_mode,
            content,
            mask: built.mask,
        };
        if let Some(level) = levels.last_mut() {
            level.push(layer);
        }
    }
    // Dividers without their group record: their layers join the level below.
    while levels.len() > 1 {
        let orphans = levels.pop().unwrap_or_default();
        if let Some(level) = levels.last_mut() {
            level.extend(orphans);
        }
    }
    let layers = levels.pop().unwrap_or_default();
    let blend_space = if header.depth == 32 {
        BlendSpace::Linear
    } else {
        BlendSpace::Perceptual
    };
    let document = Document::restore(
        canvas,
        slopshop_core::color::WORKING_SPACE,
        blend_space,
        layers,
        next_id,
    )
    .map_err(|e| corrupt(&format!("layers: {e:?}")))?;
    let layer_warnings = document
        .all_layers()
        .map(|layer| notes_of.remove(&layer.id).unwrap_or_default())
        .collect();
    Ok(Some(ImportedLayers {
        document,
        warnings,
        layer_warnings,
    }))
}

/// One layer record. Its tagged blocks tell groups, adjustments and what was rendered.
fn read_record<R: Read + Seek>(input: &mut Input<R>, max_side: u32) -> Result<Record, ImportError> {
    let bounds = Bounds::parse(&input.bytes::<16>()?).check(max_side)?;
    let channel_count = input.u16()?;
    if channel_count > MAX_LAYER_CHANNELS {
        return Err(corrupt("too many layer channels"));
    }
    let mut channels = Vec::with_capacity(usize::from(channel_count));
    for _ in 0..channel_count {
        let id = i16::from_be_bytes(input.bytes()?);
        channels.push((id, input.length()?));
    }
    if &input.bytes::<4>()? != b"8BIM" {
        return Err(corrupt("invalid layer record"));
    }
    let blend_key = input.bytes::<4>()?;
    let [opacity, clipping, flags, _filler] = input.bytes::<4>()?;
    let extra_len = u64::from(input.u32()?);
    let extra = input.vec(extra_len)?;
    let mut data = Slice {
        data: &extra,
        at: 0,
    };

    let mask_len = data.u32()? as usize;
    let mask_data = data.take(mask_len)?;
    let blend_ranges = {
        let len = data.u32()? as usize;
        data.take(len)?
    };
    let name_len = usize::from(data.u8()?);
    let pascal_name: String = data
        .take(name_len)?
        .iter()
        .map(|&b| char::from(b))
        .collect();
    // The name is padded to a multiple of 4 bytes, its count byte included.
    data.skip((1 + name_len).next_multiple_of(4) - 1 - name_len);

    let mut record = Record {
        bounds,
        channels,
        blend_key,
        opacity,
        fill: 255,
        clipping: clipping != 0,
        hidden: flags & 2 != 0,
        mask: None,
        name: pascal_name,
        kind: Kind::Pixels,
        // Each range is a (black, white) pair of 2-byte values: 0–255 is the default.
        styles: !blend_ranges
            .chunks(4)
            .all(|range| range == [0, 0, 255, 255]),
        rasterized: false,
        masks_simplified: false,
        solid_color: None,
        adjustment: None,
        adjustment_approximated: false,
    };
    let mut vector_mask = false;
    let mut section_key: Option<[u8; 4]> = None;
    let mut adjustment = false;
    let mut fill_layer = false;
    let mut brightness_contrast: Option<(Option<i32>, Option<i32>)> = None;
    while let Some((key, block)) = data.block(input.big) {
        match &key {
            b"luni" if block.len() >= 4 => {
                let count = u32::from_be_bytes([block[0], block[1], block[2], block[3]]) as usize;
                let units: Vec<u16> = block[4..]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .take(count)
                    .map(|b| u16::from_be_bytes(*b))
                    .collect();
                let name = String::from_utf16_lossy(&units);
                record.name = name.trim_end_matches('\0').to_owned();
            }
            b"lsct" | b"lsdk" if block.len() >= 4 => {
                match u32::from_be_bytes([block[0], block[1], block[2], block[3]]) {
                    1 | 2 => record.kind = Kind::GroupStart,
                    3 => record.kind = Kind::GroupEnd,
                    _ => {}
                }
                // A group's own blend mode (`pass` for pass-through) follows, when present.
                if block.len() >= 12 && &block[4..8] == b"8BIM" {
                    section_key = block[8..12].try_into().ok();
                }
            }
            b"iOpa" if !block.is_empty() => record.fill = block[0],
            key if STYLE_KEYS.contains(&key) => record.styles = true,
            b"vmsk" | b"vsms" => {
                vector_mask = true;
                record.rasterized = true;
            }
            key if RASTERIZED_KEYS.contains(&key) => record.rasterized = true,
            b"SoCo" => {
                fill_layer = true;
                record.solid_color = solid_color(block);
            }
            key if FILL_KEYS.contains(&key) => fill_layer = true,
            key if ADJUSTMENT_KEYS.contains(&key) => {
                adjustment = true;
                if let Some((parsed, approximated)) = read_adjustment(key, block) {
                    record.adjustment = Some(parsed);
                    record.adjustment_approximated |= approximated;
                }
            }
            // Brightness/Contrast's descriptor: current Photoshop versions keep the values
            // there (`brit` stays at 0); the legacy mode is reproduced approximately.
            b"CgEd" => {
                brightness_contrast = Some((
                    descriptor_long(block, b"Brgh"),
                    descriptor_long(block, b"Cntr"),
                ));
                if descriptor_bool(block, b"useLegacy").unwrap_or(false) {
                    record.adjustment_approximated = true;
                }
            }
            _ => {}
        }
    }
    // The mask: the rectangle, default color and flags. With a real (pixel) mask beside a
    // vector mask, the real mask's flags, default color and rectangle follow; then, when flag
    // bit 4 is set, the mask parameters (density and feather).
    if mask_data.len() >= 18 {
        let flags = mask_data[17];
        let user = Mask {
            bounds: Bounds::parse(&mask_data[..16]).check(max_side)?,
            default: mask_data[16],
            disabled: flags & 2 != 0,
        };
        let has = |id: i16| record.channels.iter().any(|&(c, _)| c == id);
        let real = if mask_data.len() >= 36 && has(REAL_USER_MASK) {
            Some(Mask {
                bounds: Bounds::parse(&mask_data[20..36]).check(max_side)?,
                default: mask_data[19],
                disabled: mask_data[18] & 2 != 0,
            })
        } else {
            None
        };
        record.mask = match real {
            Some(real) => Some((REAL_USER_MASK, real)),
            None if has(USER_MASK) => Some((USER_MASK, user)),
            None => None,
        };
        let parameters = if real.is_some() { 36 } else { 18 };
        if flags & 0x10 != 0 && mask_parameters_set(&mask_data[parameters.min(mask_data.len())..]) {
            record.masks_simplified = true;
        }
        // The pixel mask was taken: the vector mask beside it is left out.
        if real.is_some() && vector_mask {
            record.masks_simplified = true;
        }
    }

    if let (
        Some(Adjustment::BrightnessContrast {
            brightness,
            contrast,
        }),
        Some((new_brightness, new_contrast)),
    ) = (&mut record.adjustment, brightness_contrast)
    {
        if let Some(v) = new_brightness {
            *brightness = (v as f32).clamp(-150.0, 150.0);
        }
        if let Some(v) = new_contrast {
            *contrast = (v as f32).clamp(-50.0, 100.0);
        }
    }
    if record.kind == Kind::GroupStart
        && let Some(key) = section_key
    {
        record.blend_key = key;
    }
    if record.kind == Kind::Pixels {
        let empty = bounds.width() * bounds.height() == 0;
        // A shape whose outline is only a vector mask (not rendered to a mask channel) would
        // need vector rendering.
        let vector_only = vector_mask && record.mask.is_none();
        if adjustment {
            record.kind = if record.adjustment.is_some() {
                Kind::Adjustment
            } else {
                Kind::Skipped
            };
        } else if fill_layer && empty {
            record.kind = match record.solid_color {
                Some(_) if !vector_only => Kind::SolidColor,
                _ => Kind::Skipped,
            };
        } else if fill_layer {
            record.rasterized = true;
        }
    }
    Ok(record)
}

/// A Gradient Map's `grdm` block (Adobe's layout): version, reverse, dither, a Unicode name,
/// the color stops (location 0–4096, midpoint, mode, a color: space and four components), the
/// transparency stops, then the smoothness among the noise settings. Approximated (reported)
/// when the gradient is smooth (Photoshop's smoothness above 0), has midpoints off the middle,
/// transparency, or colors in a space other than RGB and gray.
fn gradient_map(block: &[u8]) -> Option<(Adjustment, bool)> {
    use slopshop_core::gradient::{GRADIENT_LOCATIONS, GRADIENT_STOPS, Gradient, GradientStop};
    let u16_at = |at: usize| {
        block
            .get(at..at + 2)
            .map(|b| u16::from_be_bytes([b[0], b[1]]))
    };
    let u32_at = |at: usize| {
        block
            .get(at..at + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    };
    let reverse = *block.get(2)? != 0;
    let name = u32_at(4)? as usize;
    let mut at = 8usize.checked_add(name.checked_mul(2)?)?;
    let count = usize::from(u16_at(at)?);
    at += 2;
    let mut approximated = false;
    let mut stops = Vec::with_capacity(count);
    for _ in 0..count {
        let location = u32_at(at)?.min(u32::from(GRADIENT_LOCATIONS)) as u16;
        let midpoint = u32_at(at + 4)?;
        let space = u16_at(at + 10)?;
        let c = [u16_at(at + 12)?, u16_at(at + 14)?, u16_at(at + 16)?];
        let byte = |v: u16| (u32::from(v) * 255).div_ceil(65535).min(255) as u8;
        let color = match space {
            0 => c.map(byte),
            // Gray: 0–10000, from white.
            8 => [(255 - (u32::from(c[0].min(10000)) * 255 / 10000)) as u8; 3],
            _ => {
                approximated = true;
                [0; 3]
            }
        };
        approximated |= midpoint != 50;
        stops.push(GradientStop { location, color });
        at += 20;
    }
    let transparency = usize::from(u16_at(at)?);
    at += 2;
    for _ in 0..transparency {
        approximated |= u16_at(at + 8)? < 255;
        at += 10;
    }
    // Expansion count, then the interpolation (smoothness, 0–4096).
    approximated |= u16_at(at + 2).is_some_and(|smoothness| smoothness > 0);
    stops.sort_by_key(|s| s.location);
    if stops.len() > GRADIENT_STOPS {
        stops.truncate(GRADIENT_STOPS);
        approximated = true;
    }
    if stops.len() == 1 {
        stops.push(GradientStop {
            location: GRADIENT_LOCATIONS,
            ..stops[0]
        });
    }
    let gradient = Gradient::new(&stops)?;
    Some((Adjustment::GradientMap { gradient, reverse }, approximated))
}

/// The adjustment of an adjustment layer's block, and whether it is only approximated (settings
/// this version leaves out: Hue/Saturation color ranges, Gradient Map's smoothness); `None` for
/// adjustments not reproduced yet, or damaged blocks (the layer is then skipped).
fn read_adjustment(key: &[u8; 4], block: &[u8]) -> Option<(Adjustment, bool)> {
    let u16_at = |at: usize| {
        block
            .get(at..at + 2)
            .map(|b| u16::from_be_bytes([b[0], b[1]]))
    };
    let i16_at = |at: usize| {
        block
            .get(at..at + 2)
            .map(|b| i16::from_be_bytes([b[0], b[1]]))
    };
    let f32_at = |at: usize| {
        block
            .get(at..at + 4)
            .map(|b| f32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    };
    let u32_at = |at: usize| {
        block
            .get(at..at + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    };
    let adjustment = match key {
        // Version, then records of (input black, input white, output black, output white,
        // gamma × 100), 0–255: the composite first, then each channel.
        b"levl" => {
            let record = |i: usize| -> Option<[u16; 5]> {
                let at = 2 + i * 10;
                Some([
                    u16_at(at)?,
                    u16_at(at + 2)?,
                    u16_at(at + 4)?,
                    u16_at(at + 6)?,
                    u16_at(at + 8)?,
                ])
            };
            let unit = |v: u16| f32::from(v.min(255)) / 255.0;
            let settings = |[ib, iw, ob, ow, g]: [u16; 5]| {
                [
                    unit(ib),
                    unit(iw),
                    (f32::from(g) / 100.0).clamp(0.01, 9.99),
                    unit(ob),
                    unit(ow),
                ]
            };
            let [input_black, input_white, gamma, output_black, output_white] =
                settings(record(0)?);
            // Red, green and blue (a missing record leaves its channel as it is).
            let channels = [1, 2, 3]
                .map(|i| record(i).map_or(slopshop_core::adjust::LEVELS_IDENTITY, settings));
            let levels = Adjustment::Levels {
                input_black,
                input_white,
                gamma,
                output_black,
                output_white,
                channels,
            };
            if levels.is_valid() {
                (levels, false)
            } else {
                // A channel out of range (its input black above its white): the composite only.
                (
                    Adjustment::Levels {
                        input_black,
                        input_white,
                        gamma,
                        output_black,
                        output_white,
                        channels: [slopshop_core::adjust::LEVELS_IDENTITY; 3],
                    },
                    true,
                )
            }
        }
        // Version, then exposure, offset and gamma as big-endian floats.
        b"expA" => (
            Adjustment::Exposure {
                exposure: f32_at(2)?,
                offset: f32_at(6)?,
                gamma: f32_at(10)?,
            },
            false,
        ),
        // Version, colorize, padding, colorization (3 × i16), master (hue, saturation,
        // lightness), then six color ranges (4 × i16 of range, 3 × i16 of settings).
        b"hue2" => {
            if block.get(2).copied()? != 0 {
                // Colorize is not reproduced yet.
                return None;
            }
            let ranges = (0..6).any(|i| {
                let at = 16 + i * 14 + 8;
                (0..3).any(|k| i16_at(at + k * 2).is_some_and(|v| v != 0))
            });
            (
                Adjustment::HueSaturation {
                    hue: f32::from(i16_at(10)?),
                    saturation: f32::from(i16_at(12)?),
                    lightness: f32::from(i16_at(14)?),
                },
                ranges,
            )
        }
        // Brightness, contrast (i16), then the mean and Lab-only settings.
        b"brit" => (
            Adjustment::BrightnessContrast {
                brightness: f32::from(i16_at(0)?).clamp(-150.0, 150.0),
                contrast: f32::from(i16_at(2)?).clamp(-50.0, 100.0),
            },
            false,
        ),
        // A descriptor with `vibrance` and `Strt` (saturation) as longs.
        b"vibA" => (
            Adjustment::Vibrance {
                vibrance: descriptor_long(block, b"vibrance").unwrap_or(0) as f32,
                saturation: descriptor_long(block, b"Strt").unwrap_or(0) as f32,
            },
            false,
        ),
        b"nvrt" => (Adjustment::Invert, false),
        b"grdm" => gradient_map(block)?,
        // The number of levels.
        b"post" => (
            Adjustment::Posterize {
                levels: f32::from(u16_at(0)?.clamp(2, 255)),
            },
            false,
        ),
        // The level, 1–255.
        b"thrs" => (
            Adjustment::Threshold {
                level: f32::from(u16_at(0)?.clamp(1, 255)) / 255.0,
            },
            false,
        ),
        // A descriptor: the six weights (longs), `useTint` and `tintColor` (RGB, 0–255). The
        // tint becomes its hue and saturation (as Photoshop's dialog shows them); tinting
        // itself is approximated.
        b"blwh" => {
            let weight =
                |key: &[u8]| descriptor_long(block, key).map(|v| (v as f32).clamp(-200.0, 300.0));
            let weights = [
                weight(b"Rd  ")?,
                weight(b"Yllw")?,
                weight(b"Grn ")?,
                weight(b"Cyn ")?,
                weight(b"Bl  ")?,
                weight(b"Mgnt")?,
            ];
            let tint = descriptor_bool(block, b"useTint").unwrap_or(false);
            let channel = |key: &[u8]| {
                descriptor_value(block, key, b"doub", 8)
                    .and_then(|b| b.try_into().ok())
                    .map(|b| f64::from_be_bytes(b) / 255.0)
            };
            let (tint_hue, tint_saturation) =
                match (channel(b"Rd  "), channel(b"Grn "), channel(b"Bl  ")) {
                    (Some(r), Some(g), Some(b)) => tint_of([r, g, b]),
                    _ => (42.0, 20.0),
                };
            (
                Adjustment::BlackWhite {
                    weights,
                    tint,
                    tint_hue,
                    tint_saturation,
                },
                tint,
            )
        }
        // Shadows, midtones, highlights (cyan–red, magenta–green, yellow–blue as i16), then
        // preserve luminosity (a byte).
        b"blnc" => {
            let range = |r: usize| -> Option<[f32; 3]> {
                let v =
                    |k: usize| i16_at((r * 3 + k) * 2).map(|v| f32::from(v).clamp(-100.0, 100.0));
                Some([v(0)?, v(1)?, v(2)?])
            };
            (
                Adjustment::ColorBalance {
                    shadows: range(0)?,
                    midtones: range(1)?,
                    highlights: range(2)?,
                    preserve_luminosity: block.get(18).copied()? != 0,
                },
                false,
            )
        }
        // Version 2: a color space and four u16 components (RGB 0–65535, or Lab: L 0–10000, a
        // and b in hundredths); version 3: an XYZ color. Then the density (u32, %) and
        // preserve luminosity (a byte). The color is kept within sRGB.
        b"phfl" => {
            let default = [236.0 / 255.0, 138.0 / 255.0, 0.0];
            let (color, approximated, rest) = match u16_at(0)? {
                2 => {
                    let c = [u16_at(4)?, u16_at(6)?, u16_at(8)?];
                    match u16_at(2)? {
                        0 => (c.map(|v| f64::from(v) / 65535.0), false, 12),
                        7 => {
                            let signed = |v: u16| f64::from(v as i16) / 100.0;
                            let lab = [f64::from(c[0]) / 100.0, signed(c[1]), signed(c[2])];
                            (crate::lab::lab_to_srgb(lab), false, 12)
                        }
                        // HSB, CMYK, gray…: not converted yet.
                        _ => (default, true, 12),
                    }
                }
                // HACK(no sample file): the scale of version 3's XYZ is not documented; the
                // default color stands in, reported as approximated.
                3 => (default, true, 14),
                _ => return None,
            };
            let density = u32_at(rest)?.min(100) as f32;
            (
                Adjustment::PhotoFilter {
                    color: color.map(|v| v.clamp(0.0, 1.0) as f32),
                    density,
                    preserve_luminosity: block.get(rest + 4).copied()? != 0,
                },
                approximated,
            )
        }
        // Version, monochrome, then a record per output channel (red, green, blue, and one
        // unused in RGB documents): five i16 weights in %, the inputs red, green, blue, an
        // unused one, then the constant. Monochrome uses the first record.
        b"mixr" => {
            let row = |k: usize| -> Option<[f32; 4]> {
                let v = |i: usize| {
                    i16_at(4 + k * 10 + i * 2).map(|v| f32::from(v).clamp(-200.0, 200.0))
                };
                Some([v(0)?, v(1)?, v(2)?, v(4)?])
            };
            (
                Adjustment::ChannelMixer {
                    red: row(0)?,
                    green: row(1)?,
                    blue: row(2)?,
                    monochrome: u16_at(2)? != 0,
                },
                false,
            )
        }
        // A map flag, the version, a bitmap of the curves present (composite, red, green,
        // blue, then channels RGB documents do not have), then each curve: a point count and
        // points as (output, input), 0–255. Drawn ("map") curves are not read.
        b"curv" => {
            if block.first().copied()? != 0 || !matches!(u16_at(1)?, 1 | 4) {
                return None;
            }
            let present = u32_at(3)?;
            let mut curves = [Curve::IDENTITY; 4];
            let mut at = 7;
            for channel in (0..32).filter(|c| present & (1 << c) != 0) {
                let count = usize::from(u16_at(at)?);
                let points = (0..count)
                    .map(|i| {
                        let (output, input) = (u16_at(at + 2 + i * 4)?, u16_at(at + 4 + i * 4)?);
                        Some([input.min(255) as u8, output.min(255) as u8])
                    })
                    .collect::<Option<Vec<[u8; 2]>>>()?;
                at += 2 + count * 4;
                if channel < 4 {
                    // More points than SlopShop keeps: the layer is left out.
                    curves[channel] = Curve::new(&points)?;
                }
            }
            let [rgb, red, green, blue] = curves;
            (
                Adjustment::Curves {
                    rgb,
                    red,
                    green,
                    blue,
                },
                false,
            )
        }
        _ => return None,
    };
    adjustment.0.is_valid().then_some(adjustment)
}

/// A Black & White tint color (RGB in [0, 1]) as the hue (degrees) and saturation (%, as in
/// HSB) Photoshop's dialog shows.
fn tint_of([r, g, b]: [f64; 3]) -> (f32, f32) {
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let chroma = max - min;
    if chroma <= 0.0 || max <= 0.0 {
        return (0.0, 0.0);
    }
    let h = if max == r {
        ((g - b) / chroma).rem_euclid(6.0)
    } else if max == g {
        (b - r) / chroma + 2.0
    } else {
        (r - g) / chroma + 4.0
    };
    (
        (h * 60.0).round() as f32 % 360.0,
        (chroma / max * 100.0).round() as f32,
    )
}

/// A descriptor item `key` of type `ty`: its value's bytes (found by key, as for `SoCo`). A
/// 4-character key is written after a zero length, a longer one after its length.
fn descriptor_value<'a>(block: &'a [u8], key: &[u8], ty: &[u8; 4], len: usize) -> Option<&'a [u8]> {
    let mut pattern = Vec::with_capacity(4 + key.len() + 4);
    let length = if key.len() == 4 { 0 } else { key.len() as u32 };
    pattern.extend(length.to_be_bytes());
    pattern.extend(key);
    pattern.extend(ty);
    let at = block.windows(pattern.len()).position(|w| w == pattern)? + pattern.len();
    block.get(at..at + len)
}

fn descriptor_long(block: &[u8], key: &[u8]) -> Option<i32> {
    descriptor_value(block, key, b"long", 4).map(|b| i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn descriptor_bool(block: &[u8], key: &[u8]) -> Option<bool> {
    descriptor_value(block, key, b"bool", 1).map(|b| b[0] != 0)
}

/// The color of a solid color fill layer from its `SoCo` block, encoded (0–1): its red, green
/// and blue values (0–255), or for gray documents its gray ink percentage.
fn solid_color(block: &[u8]) -> Option<[f32; 3]> {
    // HACK(no descriptor parser yet): the values are found by their key and type in the
    // descriptor instead of walking it; a descriptor parser comes with adjustment layers.
    let value = |key: &[u8; 8]| -> Option<f64> {
        let at = block.windows(8).position(|w| w == key)? + 8;
        Some(f64::from_be_bytes(block.get(at..at + 8)?.try_into().ok()?))
    };
    let unit = |v: f64| v.clamp(0.0, 1.0) as f32;
    if let (Some(r), Some(g), Some(b)) =
        (value(b"Rd  doub"), value(b"Grn doub"), value(b"Bl  doub"))
    {
        return Some([r, g, b].map(|v| unit(v / 255.0)));
    }
    Some([unit(1.0 - value(b"Gry doub")? / 100.0); 3])
}

/// Whether mask parameters (a flags byte, then for each bit set: user density `u8`, user
/// feather `f64`, vector density `u8`, vector feather `f64`) differ from the defaults (full
/// density, no feather).
fn mask_parameters_set(data: &[u8]) -> bool {
    let Some((&flags, mut rest)) = data.split_first() else {
        return false;
    };
    for bit in 0..4 {
        if flags & (1 << bit) == 0 {
            continue;
        }
        if bit % 2 == 0 {
            let Some((&density, next)) = rest.split_first() else {
                return false;
            };
            if density != 255 {
                return true;
            }
            rest = next;
        } else {
            let Some((feather, next)) = rest.split_first_chunk::<8>() else {
                return false;
            };
            if f64::from_be_bytes(*feather) != 0.0 {
                return true;
            }
            rest = next;
        }
    }
    false
}

/// A byte slice read front to back.
struct Slice<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Slice<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], ImportError> {
        let data = self
            .data
            .get(self.at..self.at.saturating_add(len))
            .ok_or_else(|| corrupt("truncated layer record"))?;
        self.at += len;
        Ok(data)
    }

    fn u8(&mut self) -> Result<u8, ImportError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, ImportError> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn skip(&mut self, len: usize) {
        self.at = self.at.saturating_add(len).min(self.data.len());
    }

    /// The next tagged block: key and data. `None` at the end or at anything unexpected
    /// (skipping up to 3 padding bytes first, as writers pad differently).
    fn block(&mut self, big: bool) -> Option<([u8; 4], &'a [u8])> {
        let pad = (0..4).find(|&pad| {
            self.data
                .get(self.at + pad..self.at + pad + 4)
                .is_some_and(|s| s == b"8BIM" || s == b"8B64")
        })?;
        self.at += pad + 4;
        let key: [u8; 4] = self.take(4).ok()?.try_into().ok()?;
        let len = if big && LONG_KEYS.contains(&&key) {
            let b = self.take(8).ok()?;
            usize::try_from(u64::from_be_bytes(b.try_into().ok()?)).ok()?
        } else {
            self.u32().ok()? as usize
        };
        let data = self.take(len).ok()?;
        Some((key, data))
    }
}

/// Photoshop's blend mode keys.
fn blend_mode(key: &[u8; 4]) -> Option<BlendMode> {
    Some(match key {
        b"norm" => BlendMode::Normal,
        b"diss" => BlendMode::Dissolve,
        b"dark" => BlendMode::Darken,
        b"mul " => BlendMode::Multiply,
        b"idiv" => BlendMode::ColorBurn,
        b"lbrn" => BlendMode::LinearBurn,
        b"dkCl" => BlendMode::DarkerColor,
        b"lite" => BlendMode::Lighten,
        b"scrn" => BlendMode::Screen,
        b"div " => BlendMode::ColorDodge,
        b"lddg" => BlendMode::LinearDodge,
        b"lgCl" => BlendMode::LighterColor,
        b"over" => BlendMode::Overlay,
        b"sLit" => BlendMode::SoftLight,
        b"hLit" => BlendMode::HardLight,
        b"vLit" => BlendMode::VividLight,
        b"lLit" => BlendMode::LinearLight,
        b"pLit" => BlendMode::PinLight,
        b"hMix" => BlendMode::HardMix,
        b"diff" => BlendMode::Difference,
        b"smud" => BlendMode::Exclusion,
        b"fsub" => BlendMode::Subtract,
        b"fdiv" => BlendMode::Divide,
        b"hue " => BlendMode::Hue,
        b"sat " => BlendMode::Saturation,
        b"colr" => BlendMode::Color,
        b"lum " => BlendMode::Luminosity,
        _ => return None,
    })
}

/// The modes where Photoshop applies the fill opacity differently from the opacity.
fn fill_is_not_opacity(mode: BlendMode) -> bool {
    matches!(
        mode,
        BlendMode::ColorBurn
            | BlendMode::LinearBurn
            | BlendMode::ColorDodge
            | BlendMode::LinearDodge
            | BlendMode::VividLight
            | BlendMode::LinearLight
            | BlendMode::HardMix
            | BlendMode::Difference
    )
}

/// What decoding a layer needs to know about the document.
struct Context {
    canvas: Size,
    depth: u16,
    colors: u16,
    sample: SampleType,
    space: ColorSpace,
    /// The space of fill colors: the profile's, encoded even in 32-bit documents.
    fill_space: ColorSpace,
    big: bool,
}

impl Context {
    /// Channels used: colors, transparency, and the mask chosen.
    fn wants(&self, record: &Record, id: i16) -> bool {
        (0..self.colors as i16).contains(&id)
            || id == TRANSPARENCY
            || record.mask.is_some_and(|(mask, _)| mask == id)
    }
}

/// Width and height of a channel: the mask's bounds for a mask, else the layer's.
fn channel_dims(record: &Record, id: i16) -> (usize, usize) {
    match record.mask {
        Some((mask, m)) if mask == id => (m.bounds.width(), m.bounds.height()),
        _ => (record.bounds.width(), record.bounds.height()),
    }
}

/// The stored bytes of one channel, refusing lengths no compression of it could reach.
fn read_channel<R: Read + Seek>(
    input: &mut Input<R>,
    len: u64,
    (width, height): (usize, usize),
    context: &Context,
) -> Result<Vec<u8>, ImportError> {
    let plane = (width * height) as u64 * u64::from(context.depth / 8);
    // Compression field, RLE row counts, and PackBits' or zlib's worst expansion.
    let limit = 2 + height as u64 * 4 + plane * 2 + 1024;
    if len > limit {
        return Err(corrupt("layer channel too large"));
    }
    let mut data = vec![0u8; len as usize];
    input.r.read_exact(&mut data)?;
    Ok(data)
}

/// One channel as `height` big-endian rows.
fn decode_channel(
    data: &[u8],
    (width, height): (usize, usize),
    context: &Context,
) -> Result<Vec<u8>, ImportError> {
    let samples = Samples {
        width,
        height,
        depth: context.depth,
    };
    let row_bytes = samples.row_bytes();
    let mut plane = vec![0u8; row_bytes * height];
    if plane.is_empty() {
        return Ok(plane);
    }
    let (compression, rest) = data
        .split_first_chunk::<2>()
        .ok_or_else(|| corrupt("missing layer channel"))?;
    match u16::from_be_bytes(*compression) {
        0 => {
            let len = plane.len();
            plane.copy_from_slice(
                rest.get(..len)
                    .ok_or_else(|| corrupt("truncated layer channel"))?,
            );
        }
        1 => {
            let count_bytes = if context.big { 4 } else { 2 };
            let (counts, mut packed) = rest
                .split_at_checked(height * count_bytes)
                .ok_or_else(|| corrupt("truncated layer channel"))?;
            for (row, count) in plane
                .chunks_exact_mut(row_bytes)
                .zip(counts.chunks_exact(count_bytes))
            {
                let len = count.iter().fold(0usize, |n, &b| n << 8 | usize::from(b));
                let (row_data, next) = packed
                    .split_at_checked(len)
                    .ok_or_else(|| corrupt("truncated layer channel"))?;
                unpack_bits(row_data, row)?;
                packed = next;
            }
        }
        compression @ (2 | 3) => {
            let mut z = ZlibDecoder::new(rest);
            z.read_exact(&mut plane)?;
            if compression == 3 {
                for row in plane.chunks_exact_mut(row_bytes) {
                    unpredict(row, &samples);
                }
            }
            // To the end of the stream: a damaged one is an error.
            std::io::copy(&mut z, &mut std::io::sink())?;
        }
        _ => return Err(corrupt("unknown compression")),
    }
    Ok(plane)
}

/// A layer's content and mask.
struct Built {
    content: LayerContent,
    mask: Option<LayerMask>,
    /// Some of its pixels were outside the canvas.
    cropped: bool,
}

fn build_layer(
    record: &Record,
    raw: &[(i16, Vec<u8>)],
    context: &Context,
) -> Result<Built, ImportError> {
    let channel = |id: i16| {
        raw.iter()
            .find(|(c, _)| *c == id)
            .map(|(_, d)| d.as_slice())
    };
    let bounds = record.bounds;
    let visible = bounds.visible(context.canvas);
    let dims = (bounds.width(), bounds.height());
    let sample_bytes = context.sample.bytes() as usize;

    let mut planes = Vec::new();
    for c in 0..context.colors as i16 {
        let data = channel(c).ok_or_else(|| corrupt("missing layer color channel"))?;
        planes.push(decode_channel(data, dims, context)?);
    }
    let transparency = channel(TRANSPARENCY)
        .map(|data| decode_channel(data, dims, context))
        .transpose()?;
    // A layer without transparency (the background) that does not cover the canvas needs
    // one: outside its bounds is transparent.
    let alpha = transparency.is_some() || visible != context.canvas.bounds();
    let layout = match (context.colors, alpha) {
        (3, false) => ChannelLayout::Rgb,
        (3, true) => ChannelLayout::Rgba,
        (_, false) => ChannelLayout::Gray,
        (_, true) => ChannelLayout::GrayAlpha,
    };
    let format = PixelFormat {
        layout,
        sample: context.sample,
        color_space: context.space,
        alpha: AlphaMode::Straight,
    };
    let opaque = unit_sample(context.sample, 255);
    let pixel_bytes = format.bytes_per_pixel() as usize;
    let pixels = interleave(
        &planes,
        transparency.as_deref(),
        alpha.then_some(opaque.as_slice()),
        bounds,
        visible,
        sample_bytes,
        pixel_bytes,
    );
    let background = vec![0u8; pixel_bytes];
    let image = RasterImage::from_placed(context.canvas, format, visible, &pixels, &background)?;
    let cropped = u64::from(visible.width) * u64::from(visible.height)
        < (bounds.width() * bounds.height()) as u64;

    Ok(Built {
        content: LayerContent::Raster {
            stack: None,
            image: slopshop_core::stack::Pixels::ready(Arc::new(image)),
        },
        mask: build_mask(record, raw, context)?,
        cropped,
    })
}

/// A group's record: its mask (its layers come from the tree, see `read_layer_info`).
fn build_group(
    record: &Record,
    raw: &[(i16, Vec<u8>)],
    context: &Context,
) -> Result<Built, ImportError> {
    Ok(Built {
        content: LayerContent::Group {
            children: Vec::new(),
            pass_through: true,
        },
        mask: build_mask(record, raw, context)?,
        cropped: false,
    })
}

/// A solid color fill layer: a fill of its color, in the working space, with its mask.
fn build_adjustment(
    record: &Record,
    raw: &[(i16, Vec<u8>)],
    context: &Context,
) -> Result<Built, ImportError> {
    let adjustment = record
        .adjustment
        .ok_or_else(|| corrupt("adjustment layer without its adjustment"))?;
    Ok(Built {
        content: LayerContent::Adjustment { adjustment },
        mask: build_mask(record, raw, context)?,
        cropped: false,
    })
}

fn build_solid(
    record: &Record,
    raw: &[(i16, Vec<u8>)],
    context: &Context,
) -> Result<Built, ImportError> {
    let encoded = record.solid_color.unwrap_or([0.0; 3]);
    let [r, g, b] = encoded.map(|v| context.fill_space.transfer.decode(v));
    let linear = LinearRgba::new(r, g, b, 1.0);
    let color = if context.colors == 1 {
        linear
    } else {
        linear.transform(&context.fill_space.matrix_to(&WORKING_SPACE))
    };
    Ok(Built {
        content: LayerContent::Fill { color },
        mask: build_mask(record, raw, context)?,
        cropped: false,
    })
}

/// A layer's mask, from its channel.
fn build_mask(
    record: &Record,
    raw: &[(i16, Vec<u8>)],
    context: &Context,
) -> Result<Option<LayerMask>, ImportError> {
    let channel = |id: i16| {
        raw.iter()
            .find(|(c, _)| *c == id)
            .map(|(_, d)| d.as_slice())
    };
    let sample_bytes = context.sample.bytes() as usize;
    Ok(match record.mask {
        Some((id, mask)) => {
            let data = channel(id).ok_or_else(|| corrupt("missing layer mask channel"))?;
            let dims = (mask.bounds.width(), mask.bounds.height());
            let plane = decode_channel(data, dims, context)?;
            let visible = mask.bounds.visible(context.canvas);
            let pixels = interleave(
                &[plane],
                None,
                None,
                mask.bounds,
                visible,
                sample_bytes,
                sample_bytes,
            );
            let format = PixelFormat {
                layout: ChannelLayout::Gray,
                sample: context.sample,
                color_space: ColorSpace::LINEAR_SRGB,
                alpha: AlphaMode::Straight,
            };
            let background = unit_sample(context.sample, mask.default);
            let image =
                RasterImage::from_placed(context.canvas, format, visible, &pixels, &background)?;
            Some(LayerMask {
                original: None,
                image: Arc::new(image),
                enabled: !mask.disabled,
                replaces_alpha: false,
            })
        }
        None => None,
    })
}

/// Native-endian bytes of an 8-bit value (0–255) at the sample type's scale.
fn unit_sample(sample: SampleType, value: u8) -> Vec<u8> {
    match sample {
        SampleType::U16 => (u16::from(value) * 257).to_ne_bytes().to_vec(),
        SampleType::F32 | SampleType::F16 => (f32::from(value) / 255.0).to_ne_bytes().to_vec(),
        SampleType::U8 => vec![value],
    }
}

/// The `visible` part of planes covering `bounds`, as packed native-endian pixels: the planes'
/// samples, then the transparency plane, or the `opaque` sample when there is none but the
/// layout has alpha.
fn interleave(
    planes: &[Vec<u8>],
    transparency: Option<&[u8]>,
    opaque: Option<&[u8]>,
    bounds: Bounds,
    visible: Rect,
    sample_bytes: usize,
    pixel_bytes: usize,
) -> Vec<u8> {
    let (vw, vh) = (visible.width as usize, visible.height as usize);
    let mut out = vec![0u8; vw * vh * pixel_bytes];
    if out.is_empty() {
        return out;
    }
    let width = bounds.width();
    let (dx, dy) = (
        (i64::from(visible.x) - bounds.left) as usize,
        (i64::from(visible.y) - bounds.top) as usize,
    );
    for (y, row) in out.chunks_exact_mut(vw * pixel_bytes).enumerate() {
        let src_row = (dy + y) * width + dx;
        for (x, px) in row.chunks_exact_mut(pixel_bytes).enumerate() {
            let at = (src_row + x) * sample_bytes;
            let sources = planes.iter().map(Vec::as_slice).chain(transparency);
            for (c, plane) in sources.enumerate() {
                let s = &plane[at..at + sample_bytes];
                let d = &mut px[c * sample_bytes..(c + 1) * sample_bytes];
                match sample_bytes {
                    2 => d.copy_from_slice(&u16::from_be_bytes([s[0], s[1]]).to_ne_bytes()),
                    4 => d.copy_from_slice(
                        &f32::from_be_bytes([s[0], s[1], s[2], s[3]]).to_ne_bytes(),
                    ),
                    _ => d[0] = s[0],
                }
            }
            if let (None, Some(opaque)) = (transparency, opaque) {
                px[pixel_bytes - sample_bytes..].copy_from_slice(opaque);
            }
        }
    }
    out
}

/// Refuse documents whose layers would not fit in the import budget.
fn check_layers_budget(
    records: &[Record],
    canvas: Size,
    header: &Header,
    sample: SampleType,
) -> Result<(), ImportError> {
    let too_large = || ImportError::TooLarge {
        width: header.width,
        height: header.height,
    };
    let bytes_per_sample = u64::from(sample.bytes());
    let mut total = 0u64;
    for record in records.iter().filter(|r| r.kind == Kind::Pixels) {
        // Colors and alpha, plus the mask: their pixels and their share of the pyramid.
        let pixels = record.bounds.visible(canvas).size().pixel_count();
        let mask = record
            .mask
            .map_or(0, |(_, m)| m.bounds.visible(canvas).size().pixel_count());
        let channels = u64::from(header.color_channels()) + 1;
        let layer = (pixels * channels + mask) * bytes_per_sample * 4 / 3;
        total = total.checked_add(layer).ok_or_else(too_large)?;
    }
    if total > MAX_IMPORT_BYTES {
        return Err(too_large());
    }
    Ok(())
}
