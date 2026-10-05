//! Files that open as one isolated group of images (the maintainer's layout): DICOM slices and
//! FITS images under the Levels that show them (a window, a stretch: Levels maps the encoded
//! samples, declared as display values, as the file means it; nothing is cut; flattening applies
//! it to the samples), PDF pages over a white background.

use std::sync::Arc;

use slopshop_core::adjust::Adjustment;
use slopshop_core::color::{SampleType, WORKING_SPACE};
use slopshop_core::document::{Layer, LayerContent, LayerId};
use slopshop_core::{BlendMode, BlendSpace, Document};

use crate::{Decoded, ImportError, Imported, ImportedLayers, Opened};

/// A document of one isolated group named `name`: `images` (named, the first on top) and above
/// them `levels` (named), which changes those images only, wherever the group goes (another
/// document, beside other images).
pub(crate) fn layered(
    name: String,
    images: Vec<(String, Imported)>,
    levels: Option<(String, Adjustment)>,
) -> Result<Opened, ImportError> {
    let above = levels.map(|(name, adjustment)| (name, LayerContent::Adjustment { adjustment }));
    grouped(name, images, above, None)
}

/// A document of one isolated group named `name`: `below` (named), `images` (named, the first on
/// top), then `above` (named). Only the first image is shown, the others hidden (the
/// maintainer's choice: one slice, page or frame at a time). The canvas fits the largest image.
pub(crate) fn grouped(
    name: String,
    images: Vec<(String, Imported)>,
    above: Option<(String, LayerContent)>,
    below: Option<(String, LayerContent)>,
) -> Result<Opened, ImportError> {
    let size = images
        .iter()
        .map(|(_, imported)| imported.image.size())
        .reduce(|a, b| slopshop_core::Size::new(a.width.max(b.width), a.height.max(b.height)))
        .ok_or_else(|| ImportError::Decode("no image".into()))?;
    let layer = |id: u64, name: String, content: LayerContent| Layer {
        style: None,
        id: LayerId::from_raw(id),
        name,
        visible: true,
        opacity: 1.0,
        blend_mode: BlendMode::Normal,
        content,
        mask: None,
        clipped: false,
        transform: slopshop_core::Projective::IDENTITY,
    };
    let mut warnings = Vec::new();
    let mut children = Vec::with_capacity(images.len() + 2);
    let mut next = 2u64;
    if let Some((below_name, content)) = below {
        children.push(layer(next, below_name, content));
        next += 1;
    }
    // Bottom to top: the last image first.
    for (index, (image_name, imported)) in images.into_iter().enumerate().rev() {
        for warning in imported.warnings {
            if !warnings.contains(&warning) {
                warnings.push(warning);
            }
        }
        let mut image = layer(
            next,
            image_name,
            LayerContent::Raster {
                stack: None,
                image: slopshop_core::stack::Pixels::ready(Arc::new(imported.image)),
            },
        );
        image.visible = index == 0;
        children.push(image);
        next += 1;
    }
    if let Some((above_name, content)) = above {
        children.push(layer(next, above_name, content));
        next += 1;
    }
    let count = children.len() + 1;
    let group = layer(
        1,
        name,
        LayerContent::Group {
            children,
            pass_through: false,
        },
    );
    let document = Document::restore(
        size,
        WORKING_SPACE,
        BlendSpace::Perceptual,
        vec![group],
        next,
    )
    .map_err(|e| ImportError::Decode(format!("adjusted image: {e:?}")))?;
    Ok(Opened::Layers(ImportedLayers {
        document,
        warnings,
        layer_warnings: vec![Vec::new(); count],
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
/// document: the composite settings (gray images have no channels of their own).
pub(crate) fn apply_levels(levels: &Adjustment, decoded: &mut Decoded) {
    let Adjustment::Levels {
        input_black,
        input_white,
        gamma,
        output_black,
        output_white,
        ..
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
