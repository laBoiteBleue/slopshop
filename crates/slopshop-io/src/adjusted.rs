//! Images whose display needs an interpretation (DICOM windows, FITS stretches): the samples as
//! they are, and a Levels adjustment layer above them in a perceptual document, where Levels
//! maps the encoded samples (declared as display values) as the file means it. Nothing is cut;
//! flattening applies the Levels to the samples.

use std::sync::Arc;

use slopshop_core::adjust::Adjustment;
use slopshop_core::color::{SampleType, WORKING_SPACE};
use slopshop_core::document::{Layer, LayerContent, LayerId};
use slopshop_core::{BlendMode, BlendSpace, Document};

use crate::{Decoded, ImportError, Imported, ImportedLayers, Opened};

/// A document of `imported` (named `name`) and above it `levels` (named `levels_name`).
pub(crate) fn layered(
    name: String,
    imported: Imported,
    levels_name: String,
    levels: Adjustment,
) -> Result<Opened, ImportError> {
    let size = imported.image.size();
    let layer = |id: u64, name: String, content: LayerContent| Layer {
        id: LayerId::from_raw(id),
        name,
        visible: true,
        opacity: 1.0,
        blend_mode: BlendMode::Normal,
        content,
        mask: None,
        clipped: false,
        transform: slopshop_core::Affine::IDENTITY,
    };
    let layers = vec![
        layer(
            1,
            name,
            LayerContent::Raster {
                image: Arc::new(imported.image),
            },
        ),
        layer(
            2,
            levels_name,
            LayerContent::Adjustment { adjustment: levels },
        ),
    ];
    let document = Document::restore(size, WORKING_SPACE, BlendSpace::Perceptual, layers, 3)
        .map_err(|e| ImportError::Decode(format!("adjusted image: {e:?}")))?;
    Ok(Opened::Layers(ImportedLayers {
        document,
        warnings: imported.warnings,
        layer_warnings: vec![Vec::new(), Vec::new()],
    }))
}

/// The samples of `decoded` as values in [0, 1] (floats as they are), every channel.
pub(crate) fn values(decoded: &Decoded) -> Box<dyn Iterator<Item = f64> + '_> {
    match decoded.sample {
        SampleType::U8 => Box::new(decoded.pixels.iter().map(|&v| f64::from(v) / 255.0)),
        SampleType::U16 => Box::new(
            decoded
                .pixels
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| f64::from(u16::from_ne_bytes(*b)) / 65535.0),
        ),
        SampleType::F16 | SampleType::F32 => Box::new(
            decoded
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f64::from(f32::from_ne_bytes(*b))),
        ),
    }
}

/// The darkest and the lightest finite sample.
pub(crate) fn extremes(decoded: &Decoded) -> Option<(f64, f64)> {
    let (low, high) = values(decoded)
        .filter(|v| v.is_finite())
        .fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)));
    (low <= high).then_some((low, high))
}

/// Apply Levels to the samples (the flattened image), as the compositor does in a perceptual
/// document.
pub(crate) fn apply_levels(levels: &Adjustment, decoded: &mut Decoded) {
    let Adjustment::Levels {
        input_black,
        input_white,
        gamma,
        output_black,
        output_white,
    } = *levels
    else {
        return;
    };
    let (ib, iw) = (f64::from(input_black), f64::from(input_white));
    let (ob, ow) = (f64::from(output_black), f64::from(output_white));
    let inverse = 1.0 / f64::from(gamma);
    let map = |v: f64| ob + ((v - ib) / (iw - ib)).clamp(0.0, 1.0).powf(inverse) * (ow - ob);
    match decoded.sample {
        SampleType::U8 => {
            for v in &mut decoded.pixels {
                *v = (map(f64::from(*v) / 255.0) * 255.0).round() as u8;
            }
        }
        SampleType::U16 => {
            for b in decoded.pixels.as_chunks_mut::<2>().0 {
                let v = map(f64::from(u16::from_ne_bytes(*b)) / 65535.0);
                *b = ((v * 65535.0).round() as u16).to_ne_bytes();
            }
        }
        SampleType::F16 | SampleType::F32 => {
            for b in decoded.pixels.as_chunks_mut::<4>().0 {
                let v = f64::from(f32::from_ne_bytes(*b));
                // Blank (NaN) samples stay blank.
                if v.is_finite() {
                    *b = (map(v) as f32).to_ne_bytes();
                }
            }
        }
    }
}
