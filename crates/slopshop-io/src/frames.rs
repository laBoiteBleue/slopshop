//! Animated images (GIF, APNG, animated WebP) opened as one document, the maintainer's layout:
//! one isolated group named after the file, one layer per frame ("anim 3/12", the first on
//! top), only the first shown (as for PDF pages and DICOM slices). Each frame is the full
//! canvas as the animation shows it (the decoders compose the partial frames); timing is not
//! kept. Flattened opens ([`open_image`](crate::open_image)) still give the first frame.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use image::codecs::gif::GifDecoder;
use image::codecs::png::PngDecoder;
use image::codecs::webp::WebPDecoder;
use image::{AnimationDecoder, Frames, ImageFormat};
use slopshop_core::Size;
use slopshop_core::color::{ChannelLayout, SampleType};

use crate::{
    Decoded, ImportError, ImportWarning, MAX_IMPORT_BYTES, Opened, adjusted, check_budget, finish,
};

/// More frames than any animation needs: a corrupt count stops here.
const MAX_FRAMES: usize = 100_000;

/// The frames of an animated `path` as one document; `None` for a still image (or a format
/// without animation), which opens as usual.
pub(crate) fn open(path: &Path, head: &[u8]) -> Result<Option<Opened>, ImportError> {
    let Ok(format) = image::guess_format(head) else {
        return Ok(None);
    };
    if !matches!(
        format,
        ImageFormat::Gif | ImageFormat::Png | ImageFormat::WebP
    ) || !crate::is_animated(format, path)
    {
        return Ok(None);
    }
    // The first frame as a still decode sees it: its color description serves every frame.
    let still = crate::decode_generic(path, head)?;
    let decode_error = |e: image::ImageError| ImportError::Decode(e.to_string());
    let reader = BufReader::new(File::open(path)?);
    let frames: Frames<'_> = match format {
        ImageFormat::Gif => GifDecoder::new(reader).map_err(decode_error)?.into_frames(),
        ImageFormat::Png => PngDecoder::new(reader)
            .and_then(|d| d.apng())
            .map_err(decode_error)?
            .into_frames(),
        _ => WebPDecoder::new(reader)
            .map_err(decode_error)?
            .into_frames(),
    };
    let stem = path.file_stem().map_or_else(
        || "animation".to_owned(),
        |s| s.to_string_lossy().into_owned(),
    );
    let mut decoded = Vec::new();
    let mut bytes = 0u64;
    for frame in frames {
        if decoded.len() >= MAX_FRAMES {
            return Err(ImportError::Decode("too many frames".into()));
        }
        let buffer = frame.map_err(decode_error)?.into_buffer();
        let (width, height) = buffer.dimensions();
        check_budget(width, height, ChannelLayout::Rgba, SampleType::U8, 0)?;
        bytes += u64::from(width) * u64::from(height) * 4;
        if bytes > MAX_IMPORT_BYTES {
            return Err(ImportError::TooLarge { width, height });
        }
        decoded.push(Decoded {
            size: Size::new(width, height),
            layout: ChannelLayout::Rgba,
            sample: SampleType::U8,
            alpha: still.alpha,
            icc: still.icc.clone(),
            space: still.space,
            orientation: still.orientation,
            pixels: buffer.into_raw(),
            warnings: Vec::new(),
        });
    }
    let count = decoded.len();
    if count < 2 {
        return Ok(None);
    }
    // The still decode's warnings (a 16-bit APNG read as 8-bit, for instance) apply to all.
    let warnings: Vec<ImportWarning> = still
        .warnings
        .into_iter()
        .filter(|w| *w != ImportWarning::FirstFrameOnly)
        .collect();
    let mut images = Vec::with_capacity(count);
    for (i, mut frame) in decoded.into_iter().enumerate() {
        if i == 0 {
            frame.warnings = warnings.clone();
        }
        images.push((format!("{stem} {}/{count}", i + 1), finish(frame)?));
    }
    adjusted::grouped(stem, images, None, None).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use slopshop_core::document::LayerContent;

    fn gif(frames: &[[u8; 4]]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "slopshop-frames-{}-{}.gif",
            std::process::id(),
            frames.len()
        ));
        let mut out = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut out);
            for color in frames {
                let frame =
                    image::Frame::new(image::RgbaImage::from_pixel(3, 2, image::Rgba(*color)));
                encoder.encode_frame(frame).unwrap();
            }
        }
        std::fs::write(&path, out).unwrap();
        path
    }

    #[test]
    fn animations_open_as_one_group_of_frames_the_first_shown() {
        let path = gif(&[[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]]);
        let opened = crate::open_file(&path);
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        std::fs::remove_file(&path).ok();
        let Ok(Opened::Layers(layers)) = opened else {
            panic!("expected layers: {opened:?}");
        };
        let document = layers.document;
        assert_eq!(document.layers().len(), 1);
        let group = &document.layers()[0];
        assert_eq!(group.name, stem);
        let LayerContent::Group {
            children,
            pass_through: false,
        } = &group.content
        else {
            panic!("expected an isolated group");
        };
        // Bottom to top: the last frame first; only the top (first) frame is shown.
        let names: Vec<&str> = children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            [
                format!("{stem} 3/3"),
                format!("{stem} 2/3"),
                format!("{stem} 1/3")
            ]
        );
        let shown: Vec<bool> = children.iter().map(|c| c.visible).collect();
        assert_eq!(shown, [false, false, true]);
        assert!(layers.warnings.is_empty(), "{:?}", layers.warnings);
    }

    #[test]
    fn stills_and_flattened_opens_are_as_before() {
        let still = gif(&[[255, 0, 0, 255]]);
        let opened = crate::open_file(&still);
        std::fs::remove_file(&still).ok();
        assert!(matches!(opened, Ok(Opened::Image(_))));
        let animated = gif(&[[255, 0, 0, 255], [0, 255, 0, 255]]);
        let flattened = crate::open_image(&animated);
        std::fs::remove_file(&animated).ok();
        assert_eq!(flattened.unwrap().warnings, [ImportWarning::FirstFrameOnly]);
    }
}
