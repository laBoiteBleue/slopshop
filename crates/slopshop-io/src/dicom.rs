//! DICOM import (dicom-rs, pure Rust): the first frame, at its own precision, and for gray
//! images the display window as a Levels adjustment layer above it, so that nothing is cut
//! (the maintainer's choice, see formats.md).
//!
//! Stored values keep their precision: samples of up to 8 bits as 8-bit, up to 16 bits as
//! 16-bit, deeper ones as 32-bit float, each scaled from its bit depth to the full range (signed
//! samples offset by half the range first). The window (center and width, after the modality
//! rescale) is then a linear map of those encoded values, which is what Levels does in a
//! perceptual document. Without a window, 8-bit images are shown as they are and deeper ones
//! stretched from their darkest to their lightest value. MONOCHROME1 (the lowest value is white)
//! inverts the Levels output.
//!
//! Pixel data: native, deflated, RLE and JPEG (baseline, extended, lossless) through
//! dicom-pixeldata; JPEG 2000 through our JPEG 2000 decoder. JPEG-LS, big-endian files and
//! palette color are refused.

use std::path::Path;

use dicom_dictionary_std::tags;
use dicom_object::{DefaultDicomObject, InMemDicomObject};
use dicom_pixeldata::PixelDecoder;
use slopshop_core::Size;
use slopshop_core::adjust::Adjustment;
use slopshop_core::color::{AlphaMode, ChannelLayout, ColorSpace, SampleType};

use crate::orient::Orientation;
use crate::{
    Decoded, ImportError, ImportWarning, MAX_IMPORT_BYTES, Opened, adjusted, check_budget, finish,
};

/// Transfer syntaxes read by the JPEG 2000 decoder (JPEG 2000 and High-Throughput JPEG 2000).
const JPEG_2000: [&str; 7] = [
    "1.2.840.10008.1.2.4.90",
    "1.2.840.10008.1.2.4.91",
    "1.2.840.10008.1.2.4.92",
    "1.2.840.10008.1.2.4.93",
    "1.2.840.10008.1.2.4.201",
    "1.2.840.10008.1.2.4.202",
    "1.2.840.10008.1.2.4.203",
];
const JPEG_LS: [&str; 2] = ["1.2.840.10008.1.2.4.80", "1.2.840.10008.1.2.4.81"];
const BIG_ENDIAN: &str = "1.2.840.10008.1.2.2";

/// Whether `head` (at least 132 bytes) is a DICOM file: the "DICM" prefix after the preamble.
pub(crate) fn is_dicom(head: &[u8]) -> bool {
    head.get(128..132) == Some(b"DICM")
}

/// The file as a document: the image, and above it the window as Levels (gray images); an
/// image alone when there is nothing to adjust.
pub(crate) fn open(path: &Path) -> Result<Opened, ImportError> {
    let scan = read(path)?;
    let imported = finish(scan.decoded)?;
    let Some(window) = scan.window else {
        return Ok(Opened::Image(imported));
    };
    let name = path
        .file_stem()
        .map_or_else(|| "DICOM".to_owned(), |s| s.to_string_lossy().into_owned());
    adjusted::layered(name, imported, window.name, window.levels)
}

/// The image as shown: the window applied to the samples (reported as a flattened document).
pub(crate) fn decode(path: &Path) -> Result<Decoded, ImportError> {
    let mut scan = read(path)?;
    if let Some(window) = scan.window {
        adjusted::apply_levels(&window.levels, &mut scan.decoded);
        scan.decoded.warnings.push(ImportWarning::LayersFlattened);
    }
    Ok(scan.decoded)
}

/// The first frame's samples, and how to show them.
struct Scan {
    decoded: Decoded,
    window: Option<Window>,
}

/// The display window of a gray image, as a Levels adjustment.
struct Window {
    levels: Adjustment,
    /// "WL 40 / WW 400", or "WL / WW auto" when computed from the samples.
    name: String,
}

/// What the attributes say about the samples.
struct Format {
    rows: u32,
    columns: u32,
    samples: u16,
    bits_allocated: u16,
    bits_stored: u16,
    high_bit: u16,
    signed: bool,
    planar: bool,
    photometric: String,
}

fn read(path: &Path) -> Result<Scan, ImportError> {
    let object = dicom_object::open_file(path).map_err(|e| decode_error(&e))?;
    let syntax = object
        .meta()
        .transfer_syntax()
        .trim_end_matches('\0')
        .trim();
    if syntax == BIG_ENDIAN {
        return Err(ImportError::NotYetSupported("big-endian DICOM"));
    }
    if JPEG_LS.contains(&syntax) {
        return Err(ImportError::NotYetSupported("JPEG-LS DICOM"));
    }
    let mut format = Format {
        rows: required(&object, tags::ROWS)?,
        columns: required(&object, tags::COLUMNS)?,
        samples: int(&object, tags::SAMPLES_PER_PIXEL).unwrap_or(1),
        bits_allocated: required(&object, tags::BITS_ALLOCATED)?,
        bits_stored: 0,
        high_bit: 0,
        signed: int::<u16>(&object, tags::PIXEL_REPRESENTATION) == Some(1),
        planar: int::<u16>(&object, tags::PLANAR_CONFIGURATION) == Some(1),
        photometric: text(&object, tags::PHOTOMETRIC_INTERPRETATION)
            .ok_or_else(|| ImportError::Decode("DICOM: no photometric interpretation".into()))?,
    };
    format.bits_stored = int(&object, tags::BITS_STORED).unwrap_or(format.bits_allocated);
    format.high_bit = int(&object, tags::HIGH_BIT).unwrap_or(format.bits_stored.saturating_sub(1));
    let frames = int::<u32>(&object, tags::NUMBER_OF_FRAMES).unwrap_or(1);
    let gray = matches!(format.photometric.as_str(), "MONOCHROME1" | "MONOCHROME2");
    if format.photometric == "PALETTE COLOR" {
        return Err(ImportError::UnsupportedPixels("DICOM palette color".into()));
    }
    let (layout, channels) = match (gray, format.samples) {
        (true, 1) => (ChannelLayout::Gray, 1),
        (false, 3) => (ChannelLayout::Rgb, 3),
        _ => {
            return Err(ImportError::UnsupportedPixels(format!(
                "DICOM {} with {} samples per pixel",
                format.photometric, format.samples
            )));
        }
    };
    check_budget(
        format.columns,
        format.rows,
        layout,
        SampleType::F32,
        channels * 4,
    )?;

    let mut decoded = if JPEG_2000.contains(&syntax) {
        let mut decoded = crate::jpeg2000::decode_bytes(&jpeg2000_frame(&object, frames)?)?;
        if decoded.layout != layout || decoded.size != Size::new(format.columns, format.rows) {
            return Err(ImportError::Decode(
                "DICOM: the JPEG 2000 frame does not match the attributes".into(),
            ));
        }
        decoded.warnings.clear();
        decoded
    } else {
        let pixels = object
            .decode_pixel_data_frame(0)
            .map_err(|e| decode_error(&e))?;
        // Encapsulated color comes back as interleaved RGB.
        format.photometric = pixels.photometric_interpretation().to_string();
        format.planar = format.planar
            && pixels.planar_configuration() != dicom_pixeldata::PlanarConfiguration::Standard;
        let data = pixels.frame_data(0).map_err(|e| decode_error(&e))?;
        native(data, &format, layout)?
    };
    if frames > 1 {
        decoded.warnings.push(ImportWarning::FirstFrameOnly);
    }
    let window = if gray {
        window(&object, &format, &decoded)
    } else {
        None
    };
    let window = window.map(|(window, approximated)| {
        if approximated {
            decoded
                .warnings
                .push(ImportWarning::DicomWindowApproximated);
        }
        window
    });
    Ok(Scan { decoded, window })
}

fn decode_error(e: &dyn std::fmt::Display) -> ImportError {
    ImportError::Decode(format!("DICOM: {e}"))
}

/// An attribute: at the top level, else in the first item of the shared, then per-frame,
/// functional groups (enhanced multi-frame objects), inside `sequence`.
fn find(
    object: &DefaultDicomObject,
    sequence: Option<dicom_core::Tag>,
    tag: dicom_core::Tag,
) -> Option<&dicom_object::mem::InMemElement> {
    if let Some(element) = object.get(tag) {
        return Some(element);
    }
    let sequence = sequence?;
    [
        tags::SHARED_FUNCTIONAL_GROUPS_SEQUENCE,
        tags::PER_FRAME_FUNCTIONAL_GROUPS_SEQUENCE,
    ]
    .into_iter()
    .find_map(|groups| {
        let group: &InMemDicomObject = object.get(groups)?.items()?.first()?;
        group.get(sequence)?.items()?.first()?.get(tag)
    })
}

/// An integer attribute, if it fits `T`.
fn int<T: TryFrom<i64>>(object: &DefaultDicomObject, tag: dicom_core::Tag) -> Option<T> {
    T::try_from(object.get(tag)?.to_int::<i64>().ok()?).ok()
}

fn required<T: TryFrom<i64>>(
    object: &DefaultDicomObject,
    tag: dicom_core::Tag,
) -> Result<T, ImportError> {
    int(object, tag).ok_or_else(|| ImportError::Decode(format!("DICOM: missing {tag}")))
}

fn text(object: &DefaultDicomObject, tag: dicom_core::Tag) -> Option<String> {
    Some(object.get(tag)?.to_str().ok()?.trim().to_owned())
}

/// The first value of a numeric attribute (possibly in the functional groups).
fn first_float(
    object: &DefaultDicomObject,
    sequence: dicom_core::Tag,
    tag: dicom_core::Tag,
) -> Option<f64> {
    find(object, Some(sequence), tag)?
        .to_multi_float64()
        .ok()?
        .first()
        .copied()
        .filter(|v| v.is_finite())
}

/// The first frame's JPEG 2000 codestream: every fragment of a single-frame object, the first
/// fragment of a multi-frame one (one fragment per frame).
fn jpeg2000_frame(object: &DefaultDicomObject, frames: u32) -> Result<Vec<u8>, ImportError> {
    let fragments = object
        .get(tags::PIXEL_DATA)
        .and_then(|e| e.value().fragments())
        .ok_or_else(|| ImportError::Decode("DICOM: no encapsulated pixel data".into()))?;
    let take = if frames > 1 { 1 } else { fragments.len() };
    let size: usize = fragments.iter().take(take).map(Vec::len).sum();
    if size as u64 > MAX_IMPORT_BYTES {
        return Err(ImportError::Decode("DICOM: frame too large".into()));
    }
    let mut frame = Vec::with_capacity(size);
    for fragment in fragments.iter().take(take) {
        frame.extend_from_slice(fragment);
    }
    Ok(frame)
}

/// The sample type that holds `bits`-bit samples without loss.
fn sample_type(bits: u16) -> SampleType {
    match bits {
        0..=8 => SampleType::U8,
        9..=16 => SampleType::U16,
        _ => SampleType::F32,
    }
}

/// Native samples (little-endian, `bits_allocated` each, interleaved or planar) as the engine's,
/// each scaled from `bits_stored` to the full range, signed ones offset by half of it.
fn native(data: &[u8], format: &Format, layout: ChannelLayout) -> Result<Decoded, ImportError> {
    let bytes = match format.bits_allocated {
        8 => 1,
        16 => 2,
        32 => 4,
        other => {
            return Err(ImportError::UnsupportedPixels(format!(
                "DICOM with {other}-bit samples"
            )));
        }
    };
    let bits = format.bits_stored;
    if bits == 0 || bits > format.bits_allocated || format.high_bit >= format.bits_allocated {
        return Err(ImportError::Decode("DICOM: invalid bit depths".into()));
    }
    let shift = format.high_bit + 1 - bits;
    let mask = (1u64 << bits) - 1;
    let sign = if format.signed { 1u64 << (bits - 1) } else { 0 };
    let channels = layout.channels() as usize;
    let size = Size::new(format.columns, format.rows);
    let count = size.pixel_count() as usize * channels;
    if data.len() < count * bytes {
        return Err(ImportError::Decode("DICOM: pixel data too short".into()));
    }
    // Offset binary: flipping the sign bit of a two's complement value adds half the range.
    let value = |i: usize| -> u64 {
        let at = &data[i * bytes..(i + 1) * bytes];
        let raw = match bytes {
            1 => u64::from(at[0]),
            2 => u64::from(u16::from_le_bytes([at[0], at[1]])),
            _ => u64::from(u32::from_le_bytes([at[0], at[1], at[2], at[3]])),
        };
        ((raw >> shift) & mask) ^ sign
    };
    // Planar data stores each channel's plane in turn.
    let plane = size.pixel_count() as usize;
    let index = |i: usize| {
        if format.planar && channels > 1 {
            (i % channels) * plane + i / channels
        } else {
            i
        }
    };
    let sample = sample_type(bits);
    let full = mask as f64;
    let ycbcr = format.photometric == "YBR_FULL";
    if !ycbcr
        && !matches!(
            format.photometric.as_str(),
            "MONOCHROME1" | "MONOCHROME2" | "RGB"
        )
    {
        return Err(ImportError::UnsupportedPixels(format!(
            "DICOM {}",
            format.photometric
        )));
    }
    let pixels: Vec<u8> = match sample {
        SampleType::U8 => {
            let scale = 255.0 / full;
            let mut out: Vec<u8> = (0..count)
                .map(|i| (value(index(i)) as f64 * scale).round() as u8)
                .collect();
            if ycbcr {
                ycbcr_to_rgb(&mut out);
            }
            out
        }
        SampleType::U16 => {
            if ycbcr {
                return Err(ImportError::UnsupportedPixels(
                    "DICOM YBR_FULL deeper than 8 bits".into(),
                ));
            }
            let scale = 65535.0 / full;
            (0..count)
                .flat_map(|i| ((value(index(i)) as f64 * scale).round() as u16).to_ne_bytes())
                .collect()
        }
        SampleType::F16 | SampleType::F32 => {
            if ycbcr {
                return Err(ImportError::UnsupportedPixels(
                    "DICOM YBR_FULL deeper than 8 bits".into(),
                ));
            }
            (0..count)
                .flat_map(|i| ((value(index(i)) as f64 / full) as f32).to_ne_bytes())
                .collect()
        }
    };
    Ok(Decoded {
        size,
        layout,
        sample,
        alpha: AlphaMode::Straight,
        icc: None,
        // Stored values are display values (the window maps them linearly): encoded, float
        // samples too, which the engine would otherwise read as linear light.
        space: Some(ColorSpace::SRGB),
        orientation: Orientation::Normal,
        pixels,
        warnings: Vec::new(),
    })
}

/// Full-range YCbCr (BT.601) to RGB, in place.
fn ycbcr_to_rgb(pixels: &mut [u8]) {
    for p in pixels.as_chunks_mut::<3>().0 {
        let [y, cb, cr] = p.map(f64::from);
        let (cb, cr) = (cb - 128.0, cr - 128.0);
        let rgb = [
            y + 1.402 * cr,
            y - 0.344_136 * cb - 0.714_136 * cr,
            y + 1.772 * cb,
        ];
        *p = rgb.map(|v| v.round().clamp(0.0, 255.0) as u8);
    }
}

/// The window of a gray image as Levels, and whether it is approximated (a sigmoid or a lookup
/// table read as a linear window). `None` when the samples are shown as they are.
fn window(
    object: &DefaultDicomObject,
    format: &Format,
    decoded: &Decoded,
) -> Option<(Window, bool)> {
    let slope = first_float(
        object,
        tags::PIXEL_VALUE_TRANSFORMATION_SEQUENCE,
        tags::RESCALE_SLOPE,
    )
    .filter(|s| *s != 0.0)
    .unwrap_or(1.0);
    let intercept = first_float(
        object,
        tags::PIXEL_VALUE_TRANSFORMATION_SEQUENCE,
        tags::RESCALE_INTERCEPT,
    )
    .unwrap_or(0.0);
    let center = first_float(object, tags::FRAME_VOILUT_SEQUENCE, tags::WINDOW_CENTER);
    let width = first_float(object, tags::FRAME_VOILUT_SEQUENCE, tags::WINDOW_WIDTH);
    let function = find(
        object,
        Some(tags::FRAME_VOILUT_SEQUENCE),
        tags::VOILUT_FUNCTION,
    )
    .and_then(|e| e.to_str().ok().map(|s| s.trim().to_owned()))
    .unwrap_or_default();
    let has_lut = find(
        object,
        Some(tags::FRAME_VOILUT_SEQUENCE),
        tags::VOILUT_SEQUENCE,
    )
    .is_some();
    let inverted = format.photometric == "MONOCHROME1";

    // Stored values as the encoded values the engine holds (in [0, 1]).
    let bits = format.bits_stored.clamp(1, 32);
    let full = ((1u64 << bits) - 1) as f64;
    let offset = if format.signed {
        (1u64 << (bits - 1)) as f64
    } else {
        0.0
    };
    let encoded = |modality: f64| ((modality - intercept) / slope + offset) / full;

    let (from, to, name, approximated) = match (center, width) {
        (Some(c), Some(w)) if w > 0.0 => {
            let (from, to, approximated) = match function.as_str() {
                "LINEAR_EXACT" => (c - w / 2.0, c + w / 2.0, false),
                "SIGMOID" => (c - w / 2.0, c + w / 2.0, true),
                // LINEAR (the default) needs a width of at least 1.
                _ if w >= 1.0 => (
                    c - 0.5 - (w - 1.0) / 2.0,
                    c - 0.5 + (w - 1.0) / 2.0,
                    has_lut,
                ),
                _ => (c - w / 2.0, c + w / 2.0, true),
            };
            let name = format!("WL {} / WW {}", number(c), number(w));
            (encoded(from), encoded(to), name, approximated)
        }
        // 8-bit images without a window are shown as they are (inverted for MONOCHROME1).
        _ if bits <= 8 && !format.signed && !has_lut => {
            if !inverted {
                return None;
            }
            (0.0, 1.0, "WL / WW auto".to_owned(), false)
        }
        // Deeper ones: from the darkest to the lightest sample.
        _ => {
            let (low, high) = adjusted::extremes(decoded)?;
            (low, high, "WL / WW auto".to_owned(), has_lut)
        }
    };
    let levels = levels(from, to, inverted)?;
    Some((Window { levels, name }, approximated))
}

/// A number as short as it can be: "40", "-600", "0.5".
fn number(v: f64) -> String {
    let rounded = (v * 100.0).round() / 100.0;
    if rounded.fract() == 0.0 {
        format!("{}", rounded as i64)
    } else {
        format!("{rounded}")
    }
}

/// Levels mapping encoded `from` to black and `to` to white (the reverse if `inverted`), linearly.
/// Levels' inputs live in [0, 1]: a window reaching beyond moves its outputs instead. `None`
/// for an empty window.
fn levels(from: f64, to: f64, inverted: bool) -> Option<Adjustment> {
    let (from, to, inverted) = if from <= to {
        (from, to, inverted)
    } else {
        // A negative rescale slope reverses the window.
        (to, from, !inverted)
    };
    if !from.is_finite() || !to.is_finite() || to - from <= 0.0 {
        return None;
    }
    let map = |e: f64| ((e - from) / (to - from)).clamp(0.0, 1.0);
    let (mut input_black, mut input_white) = (from.clamp(0.0, 1.0), to.clamp(0.0, 1.0));
    let (mut output_black, mut output_white) = (map(input_black), map(input_white));
    if input_white - input_black < 1e-6 {
        // The window is beyond the samples: every value maps to the same output.
        let constant = if to <= 0.0 { 1.0 } else { 0.0 };
        (input_black, input_white) = (0.0, 1.0);
        (output_black, output_white) = (constant, constant);
    }
    if inverted {
        (output_black, output_white) = (1.0 - output_black, 1.0 - output_white);
    }
    Some(Adjustment::Levels {
        input_black: input_black as f32,
        input_white: input_white as f32,
        gamma: 1.0,
        output_black: output_black as f32,
        output_white: output_white as f32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use slopshop_core::BlendSpace;
    use slopshop_core::document::LayerContent;

    fn fixture(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/dicom")
            .join(name)
    }

    fn u16s(bytes: &[u8]) -> Vec<u16> {
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_ne_bytes(*b))
            .collect()
    }

    /// The CT pattern as stored (signed), then as the engine's 16-bit encoded values.
    fn ct() -> Vec<i32> {
        let mut out = Vec::new();
        for y in 0..48 {
            for x in 0..64 {
                out.push((x * 60 + y * 30 - 2048).clamp(-2048, 2047));
            }
        }
        out
    }

    fn ct_encoded() -> Vec<u16> {
        ct().iter()
            .map(|&v| (f64::from(v + 2048) * 65535.0 / 4095.0).round() as u16)
            .collect()
    }

    fn scan(name: &str) -> Scan {
        read(&fixture(name)).unwrap()
    }

    fn levels_of(scan: &Scan) -> [f32; 4] {
        match scan.window.as_ref().unwrap().levels {
            Adjustment::Levels {
                input_black,
                input_white,
                output_black,
                output_white,
                gamma,
            } => {
                assert_eq!(gamma, 1.0);
                [input_black, input_white, output_black, output_white]
            }
            _ => panic!("not levels"),
        }
    }

    #[test]
    fn signed_twelve_bit_samples_keep_every_value() {
        for name in ["ct.dcm", "ct-rle.dcm", "ct-j2k.dcm"] {
            let scan = scan(name);
            let d = &scan.decoded;
            assert_eq!(
                (d.layout, d.sample, d.size),
                (ChannelLayout::Gray, SampleType::U16, Size::new(64, 48)),
                "{name}"
            );
            assert_eq!(u16s(&d.pixels), ct_encoded(), "{name}");
            assert!(d.warnings.is_empty(), "{name}: {:?}", d.warnings);
        }
    }

    #[test]
    fn the_window_becomes_levels_over_the_stored_values() {
        // WL 40 / WW 400 (LINEAR): from −160 to 239 HU, stored values 864 to 1263 (HU =
        // stored − 1024), encoded with the signed offset.
        let scan = scan("ct.dcm");
        assert_eq!(scan.window.as_ref().unwrap().name, "WL 40 / WW 400");
        let [ib, iw, ob, ow] = levels_of(&scan);
        let encoded = |stored: f64| ((stored + 2048.0) / 4095.0) as f32;
        assert!((ib - encoded(864.0)).abs() < 1e-6, "{ib}");
        assert!((iw - encoded(1263.0)).abs() < 1e-6, "{iw}");
        assert_eq!((ob, ow), (0.0, 1.0));
    }

    #[test]
    fn the_flattened_image_is_the_windowed_one() {
        let decoded = decode(&fixture("ct.dcm")).unwrap();
        assert_eq!(decoded.warnings, [ImportWarning::LayersFlattened]);
        let pixels = u16s(&decoded.pixels);
        let hu = |i: usize| f64::from(ct()[i]) - 1024.0;
        // The 16-bit samples round the 12-bit ones by half a level at most; the window
        // (399 values of 4095) magnifies that about ten times.
        let tolerance = 0.5 * 4095.0 / 399.0 + 0.5;
        for (i, &v) in pixels.iter().enumerate() {
            // DICOM's LINEAR window, from 0 to 65535.
            let expected = ((hu(i) - 39.5) / 399.0 + 0.5).clamp(0.0, 1.0) * 65535.0;
            assert!(
                (f64::from(v) - expected).abs() <= tolerance,
                "{i}: {v} vs {expected}"
            );
        }
    }

    #[test]
    fn opening_gives_the_image_and_its_window_as_layers() {
        let Opened::Layers(layers) = open(&fixture("ct.dcm")).unwrap() else {
            panic!("expected layers");
        };
        let document = layers.document;
        assert_eq!(document.layers().len(), 2);
        assert_eq!(document.layers()[0].name, "ct");
        assert!(matches!(
            document.layers()[1].content,
            LayerContent::Adjustment {
                adjustment: Adjustment::Levels { .. }
            }
        ));
        assert_eq!(document.blend_space(), BlendSpace::Perceptual);
    }

    #[test]
    fn monochrome1_inverts_and_plain_8_bit_images_are_shown_as_they_are() {
        let mono1 = scan("mono1.dcm");
        assert_eq!(mono1.decoded.sample, SampleType::U8);
        assert_eq!(levels_of(&mono1), [0.0, 1.0, 1.0, 0.0]);
        let rgb = scan("rgb.dcm");
        assert!(rgb.window.is_none());
        assert!(matches!(open(&fixture("rgb.dcm")), Ok(Opened::Image(_))));
    }

    #[test]
    fn color_is_interleaved_whatever_the_planar_configuration() {
        let expected: Vec<u8> = (0..48)
            .flat_map(|y| {
                (0..64).flat_map(move |x| {
                    [
                        (x * 4 % 256) as u8,
                        (y * 5 % 256) as u8,
                        ((x + y) * 3 % 256) as u8,
                    ]
                })
            })
            .collect();
        for name in ["rgb.dcm", "rgb-planar.dcm"] {
            let scan = scan(name);
            assert_eq!(scan.decoded.layout, ChannelLayout::Rgb, "{name}");
            assert_eq!(scan.decoded.pixels, expected, "{name}");
        }
    }

    #[test]
    fn multi_frame_files_give_their_first_frame() {
        let scan = scan("frames.dcm");
        assert_eq!(scan.decoded.warnings, [ImportWarning::FirstFrameOnly]);
        assert_eq!(u16s(&scan.decoded.pixels)[1], 1000);
        // LINEAR_EXACT, 32000 / 64000: from 0 to 64000.
        let [ib, iw, ..] = levels_of(&scan);
        assert_eq!(ib, 0.0);
        assert!((iw - 64000.0 / 65535.0).abs() < 1e-6, "{iw}");
    }

    #[test]
    fn deep_samples_are_float_and_read_as_display_values() {
        let format = Format {
            rows: 1,
            columns: 2,
            samples: 1,
            bits_allocated: 32,
            bits_stored: 32,
            high_bit: 31,
            signed: false,
            planar: false,
            photometric: "MONOCHROME2".into(),
        };
        let data = [0u32, u32::MAX].map(u32::to_le_bytes).concat();
        let decoded = native(&data, &format, ChannelLayout::Gray).unwrap();
        assert_eq!(decoded.sample, SampleType::F32);
        assert_eq!(decoded.space, Some(ColorSpace::SRGB));
        let values: Vec<f32> = decoded
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_ne_bytes(*b))
            .collect();
        assert_eq!(values, [0.0, 1.0]);
    }

    #[test]
    fn windows_beyond_the_samples_move_the_outputs() {
        // From −0.5 to 0.5 in encoded values: input 0 already maps to half gray.
        let Some(Adjustment::Levels {
            input_black,
            input_white,
            output_black,
            output_white,
            ..
        }) = levels(-0.5, 0.5, false)
        else {
            panic!()
        };
        assert_eq!(
            (input_black, input_white, output_black, output_white),
            (0.0, 0.5, 0.5, 1.0)
        );
        assert!(levels(0.3, 0.3, false).is_none());
        // Entirely above the samples: everything black.
        let Some(Adjustment::Levels {
            output_black,
            output_white,
            ..
        }) = levels(2.0, 3.0, false)
        else {
            panic!()
        };
        assert_eq!((output_black, output_white), (0.0, 0.0));
    }

    #[test]
    fn unsupported_files_are_refused_with_their_reason() {
        assert!(matches!(
            read(&fixture("jpegls.dcm")),
            Err(ImportError::NotYetSupported("JPEG-LS DICOM"))
        ));
        let mut bytes = std::fs::read(fixture("ct.dcm")).unwrap();
        assert!(is_dicom(&bytes));
        bytes.truncate(bytes.len() - 2000);
        let path =
            std::env::temp_dir().join(format!("slopshop-dicom-{}-cut.dcm", std::process::id()));
        std::fs::write(&path, &bytes).unwrap();
        let result = read(&path);
        std::fs::remove_file(&path).ok();
        assert!(result.is_err());
    }
}
