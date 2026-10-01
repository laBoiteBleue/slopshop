//! Reading image files into the engine's [`RasterImage`] (ADR 0006), and writing images to
//! files ([`export`], ADR 0008).
//!
//! Decoding keeps the source as it is and is explicit about everything else:
//! - native sample types (8/16-bit integer, 16/32-bit float) and gray/color layouts are kept;
//! - the color space comes from what the file declares (ICC profile for matrix/TRC profiles,
//!   PNG cICP/sRGB/gAMA/cHRM chunks, QOI linear flag, OpenEXR chromaticities), or is assumed
//!   (sRGB for integer data, linear sRGB primaries for float data) when the file has none;
//! - EXIF orientation is applied (a lossless transform);
//! - what cannot be represented faithfully yet is refused with a clear error (CMYK, LUT-based
//!   color, exotic samples, formats not supported yet such as HEIC or RAW) or
//!   imported with a warning (first page/frame only, approximated tone curve).

mod adjusted;
mod atomic;
mod avif;
pub mod collection;
mod dicom;
pub mod export;
mod fits;
mod icc;
mod jpeg2000;
mod jxl;
mod lab;
mod orient;
pub mod pdf;
mod pfm;
mod psd;
pub mod slop;
mod svg;
mod tiff_import;
pub mod vector;

use std::fmt;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use image::{ColorType, ImageDecoder, ImageFormat, ImageReader, Limits};
use slopshop_core::color::{
    AlphaMode, ChannelLayout, ColorSpace, PixelFormat, RgbPrimaries, SampleType, TransferFunction,
};
use slopshop_core::raster::RasterError;
use slopshop_core::{Document, RasterImage, Size};

use crate::orient::Orientation;

/// Upper bound on the memory an import may need, checked from the file header before any
/// pixel is decoded. Generous on purpose (large documents are the point), but bounded so that
/// a corrupt or crafted header cannot make the process request absurd amounts of memory (an
/// allocation failure aborts the whole application). A budget based on the machine's RAM is
/// future work (out-of-core storage).
const MAX_IMPORT_BYTES: u64 = 32 * 1024 * 1024 * 1024;

/// Something the user should know about how the file was interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportWarning {
    /// The embedded ICC profile is not a matrix/TRC profile: colors are read as sRGB.
    IccProfileUnsupported,
    /// The profile's tone curve was approximated by a parametric curve.
    IccCurveApproximated,
    /// The file is animated: only the first frame was imported.
    FirstFrameOnly,
    /// The file has several pages/images: only the first was imported.
    FirstPageOnly,
    /// 64-bit float samples were stored as 32-bit floats.
    PrecisionReduced,
    /// Some float samples are infinite or NaN: they are kept, and shown as the largest finite
    /// value (NaN as 0).
    NonFiniteSamples,
    /// The file declares color information that cannot be represented yet: colors are read as
    /// sRGB (linear for float data).
    ColorInfoUnsupported,
    /// A layered document (Photoshop) was opened as its flattened image: its layers were not
    /// imported (they could not be read, or the caller asked for the image).
    LayersFlattened,
    /// Some adjustment and fill layers are not supported yet: they were left out.
    AdjustmentLayersSkipped,
    /// Some settings of adjustment layers are not supported yet (Levels per channel,
    /// Hue/Saturation color ranges, legacy Brightness/Contrast, a blend mode other than normal):
    /// the adjustment is close, not identical.
    AdjustmentsApproximated,
    /// Layer styles (effects) and advanced blending options are not supported yet: they were
    /// left out.
    LayerStylesIgnored,
    /// Text, shapes, smart objects and vector masks were imported as their pixels.
    LayersRasterized,
    /// Parts of layers lay outside the canvas: they were cropped.
    PixelsOutsideCanvas,
    /// Mask density and feather, and vector masks beside a pixel mask, are not supported yet:
    /// they were left out.
    MasksSimplified,
    /// Some content of a PDF page could not be rendered (an unsupported font or an image that
    /// failed to decode): it is missing from the page.
    PdfContentSkipped,
    /// The DICOM display window is a sigmoid or a lookup table, or is invalid: it was read as a
    /// linear window, the closest one.
    DicomWindowApproximated,
    /// FITS float samples were scaled from their own range to [0, 1].
    FitsValuesScaled,
}

impl ImportWarning {
    /// Stable identifier, translated by the UI.
    pub fn id(self) -> &'static str {
        match self {
            ImportWarning::IccProfileUnsupported => "iccProfileUnsupported",
            ImportWarning::IccCurveApproximated => "iccCurveApproximated",
            ImportWarning::FirstFrameOnly => "firstFrameOnly",
            ImportWarning::FirstPageOnly => "firstPageOnly",
            ImportWarning::PrecisionReduced => "precisionReduced",
            ImportWarning::NonFiniteSamples => "nonFiniteSamples",
            ImportWarning::ColorInfoUnsupported => "colorInfoUnsupported",
            ImportWarning::LayersFlattened => "layersFlattened",
            ImportWarning::AdjustmentLayersSkipped => "adjustmentLayersSkipped",
            ImportWarning::AdjustmentsApproximated => "adjustmentsApproximated",
            ImportWarning::LayerStylesIgnored => "layerStylesIgnored",
            ImportWarning::LayersRasterized => "layersRasterized",
            ImportWarning::PixelsOutsideCanvas => "pixelsOutsideCanvas",
            ImportWarning::MasksSimplified => "masksSimplified",
            ImportWarning::PdfContentSkipped => "pdfContentSkipped",
            ImportWarning::DicomWindowApproximated => "dicomWindowApproximated",
            ImportWarning::FitsValuesScaled => "fitsValuesScaled",
        }
    }
}

#[derive(Debug)]
pub struct Imported {
    pub image: RasterImage,
    pub warnings: Vec<ImportWarning>,
}

/// A layered file (Photoshop) opened as a document.
#[derive(Debug)]
pub struct ImportedLayers {
    pub document: Document,
    /// About the whole file.
    pub warnings: Vec<ImportWarning>,
    /// About each layer, in the order of [`Document::all_layers`].
    pub layer_warnings: Vec<Vec<ImportWarning>>,
}

/// What [`open_file`] made of a file.
#[derive(Debug)]
pub enum Opened {
    Image(Imported),
    Layers(ImportedLayers),
}

#[derive(Debug)]
pub enum ImportError {
    Io(std::io::Error),
    Decode(String),
    /// A format that is recognized but not supported yet (planned: see ADR 0006).
    NotYetSupported(&'static str),
    /// HEIC/HEIF, deliberately not supported for now (HEVC patents, ADR 0006).
    HeicUnsupported,
    /// A Photoshop document saved without its flattened image ("Maximize Compatibility" off),
    /// asked for as an image, or whose layers could not be read either.
    PsdWithoutComposite,
    /// Pixel data the engine cannot store faithfully yet (e.g. CMYK, signed integers).
    UnsupportedPixels(String),
    /// The image would need more memory than an import is allowed to use.
    TooLarge {
        width: u32,
        height: u32,
    },
    /// Not an image format SlopShop knows.
    Unrecognized,
    Raster(RasterError),
}

impl ImportError {
    /// Stable identifier, translated by the UI (`open.error.<code>`).
    pub fn code(&self) -> &'static str {
        match self {
            ImportError::Io(_) => "io",
            ImportError::Decode(_) => "decode",
            ImportError::NotYetSupported(_) => "notYetSupported",
            ImportError::HeicUnsupported => "heic",
            ImportError::PsdWithoutComposite => "psdWithoutComposite",
            ImportError::UnsupportedPixels(_) => "unsupportedPixels",
            ImportError::TooLarge { .. } => "tooLarge",
            ImportError::Unrecognized => "unrecognized",
            ImportError::Raster(_) => "internal",
        }
    }
}

/// The technical detail only: the UI puts it inside a translated message chosen by
/// [`ImportError::code`].
impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImportError::Io(e) => write!(f, "{e}"),
            ImportError::Decode(e) => write!(f, "{e}"),
            ImportError::NotYetSupported(format) => write!(f, "{format}"),
            ImportError::HeicUnsupported => write!(f, "HEIC/HEIF"),
            ImportError::PsdWithoutComposite => write!(f, "no flattened image"),
            ImportError::UnsupportedPixels(what) => write!(f, "{what}"),
            ImportError::TooLarge { width, height } => write!(f, "{width}×{height}"),
            ImportError::Unrecognized => write!(f, "unrecognized format"),
            ImportError::Raster(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ImportError {}

impl From<std::io::Error> for ImportError {
    fn from(e: std::io::Error) -> Self {
        // A truncated or inconsistent file is a damaged image, not an unreadable file.
        match e.kind() {
            std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::InvalidData => {
                ImportError::Decode(e.to_string())
            }
            _ => ImportError::Io(e),
        }
    }
}

impl From<image::ImageError> for ImportError {
    fn from(e: image::ImageError) -> Self {
        match e {
            image::ImageError::IoError(io) => io.into(),
            image::ImageError::Unsupported(u) => ImportError::UnsupportedPixels(u.to_string()),
            image::ImageError::Limits(_) => ImportError::TooLarge {
                width: 0,
                height: 0,
            },
            other => ImportError::Decode(other.to_string()),
        }
    }
}

impl From<RasterError> for ImportError {
    fn from(e: RasterError) -> Self {
        ImportError::Raster(e)
    }
}

/// What a decoder produced, before color interpretation and orientation.
pub(crate) struct Decoded {
    size: Size,
    layout: ChannelLayout,
    sample: SampleType,
    alpha: AlphaMode,
    icc: Option<Vec<u8>>,
    /// Color space declared without an ICC profile; takes precedence over `icc`.
    space: Option<ColorSpace>,
    orientation: Orientation,
    /// Tightly packed rows, native endianness.
    pixels: Vec<u8>,
    warnings: Vec<ImportWarning>,
}

/// Whether the file at `path` is a DICOM file (by its content).
pub fn is_dicom_file(path: &Path) -> std::io::Result<bool> {
    let mut head = Vec::with_capacity(132);
    File::open(path)?.take(132).read_to_end(&mut head)?;
    Ok(dicom::is_dicom(&head))
}

/// DICOM files opened together (a series): one document, every slice in one group under one
/// window, ordered by Instance Number; files that cannot be read are returned with their error.
/// Blocking and CPU-heavy: call it off the UI thread.
pub fn open_dicom_series(
    paths: &[std::path::PathBuf],
) -> Result<(Opened, Vec<(std::path::PathBuf, ImportError)>), ImportError> {
    dicom::open_series(paths)
}

/// Decode a file: a layered document (Photoshop) as its layers when the engine can hold them,
/// anything else as [`open_image`] does. Blocking and CPU-heavy: call it off the UI thread.
pub fn open_file(path: &Path) -> Result<Opened, ImportError> {
    let mut head = Vec::with_capacity(132);
    File::open(path)?.take(132).read_to_end(&mut head)?;
    if psd::is_psd(&head) {
        return psd::open(path);
    }
    if dicom::is_dicom(&head) {
        return dicom::open(path);
    }
    if fits::is_fits(&head) {
        return fits::open(path);
    }
    if pdf::is_pdf(&head) {
        return pdf::open(path);
    }
    open_image(path).map(Opened::Image)
}

/// Decode an image file; a layered document gives its flattened image. Blocking and
/// CPU-heavy: call it off the UI thread.
pub fn open_image(path: &Path) -> Result<Imported, ImportError> {
    let mut file = File::open(path)?;
    let mut head = Vec::with_capacity(4096);
    (&mut file).take(4096).read_to_end(&mut head)?;
    file.seek(SeekFrom::Start(0))?;

    if let Some(error) = not_supported_yet(&head, path) {
        return Err(error);
    }
    let decoded = if is_tiff(&head) {
        tiff_import::decode(file)?
    } else if psd::is_psd(&head) {
        drop(file);
        psd::decode(path)?
    } else if pfm::is_pfm(&head) {
        drop(file);
        pfm::decode(path)?
    } else if jxl::is_jxl(&head) {
        drop(file);
        jxl::decode(path)?
    } else if avif::is_avif(&head) {
        drop(file);
        avif::decode(path)?
    } else if jpeg2000::is_jpeg2000(&head) {
        drop(file);
        jpeg2000::decode(path)?
    } else if pdf::is_pdf(&head) {
        drop(file);
        pdf::decode(path)?
    } else if dicom::is_dicom(&head) {
        drop(file);
        dicom::decode(path)?
    } else if fits::is_fits(&head) {
        drop(file);
        fits::decode(path)?
    } else if svg::is_svg(&head, path) {
        drop(file);
        svg::decode(path)?
    } else {
        drop(file);
        decode_generic(path, &head)?
    };
    finish(decoded)
}

/// Interpret colors, orient, and build the tiled raster.
fn finish(decoded: Decoded) -> Result<Imported, ImportError> {
    let mut warnings = decoded.warnings;
    let float = decoded.sample.is_float();
    let color_space =
        resolve_color_space(float, decoded.space, decoded.icc.as_deref(), &mut warnings);
    let format = PixelFormat {
        layout: decoded.layout,
        sample: decoded.sample,
        color_space,
        alpha: decoded.alpha,
    };
    let bpp = format.bytes_per_pixel() as usize;
    // Every decoder path is checked here, before anything indexes the buffer.
    let expected = decoded.size.pixel_count() * bpp as u64;
    if decoded.pixels.len() as u64 != expected {
        return Err(ImportError::Decode(format!(
            "decoder returned {} bytes, expected {expected}",
            decoded.pixels.len()
        )));
    }
    if float && has_non_finite(&decoded.pixels, decoded.sample) {
        warnings.push(ImportWarning::NonFiniteSamples);
    }
    let (pixels, size) = orient::apply(decoded.pixels, decoded.size, bpp, decoded.orientation);
    let image = RasterImage::from_pixels(size, format, &pixels)?;
    Ok(Imported { image, warnings })
}

/// The color space of decoded samples: `space` if the decoder knows it, else the ICC profile's,
/// else sRGB for integer data and linear sRGB primaries for float data (EXR, HDR, PFM, float
/// TIFF), which is also the fallback for unsupported profiles (with a warning).
fn resolve_color_space(
    float: bool,
    space: Option<ColorSpace>,
    icc: Option<&[u8]>,
    warnings: &mut Vec<ImportWarning>,
) -> ColorSpace {
    let assumed = if float {
        ColorSpace::LINEAR_SRGB
    } else {
        ColorSpace::SRGB
    };
    match (space, icc) {
        (Some(space), _) => space,
        (None, None) => assumed,
        (None, Some(bytes)) => match icc::parse(bytes) {
            Ok(color) => {
                if color.approximated {
                    warnings.push(ImportWarning::IccCurveApproximated);
                }
                color.space
            }
            Err(_) => {
                warnings.push(ImportWarning::IccProfileUnsupported);
                assumed
            }
        },
    }
}

/// Whether float pixels contain infinities or NaNs.
fn has_non_finite(pixels: &[u8], sample: SampleType) -> bool {
    match sample {
        SampleType::F16 => pixels
            .as_chunks::<2>()
            .0
            .iter()
            .any(|b| u16::from_ne_bytes(*b) & 0x7c00 == 0x7c00),
        SampleType::F32 => pixels
            .as_chunks::<4>()
            .0
            .iter()
            .any(|b| !f32::from_ne_bytes(*b).is_finite()),
        SampleType::U8 | SampleType::U16 => false,
    }
}

/// Formats decoded through the `image` crate's individual codecs. The format comes from the
/// content, or from the extension for formats without a signature (TGA).
fn decode_generic(path: &Path, head: &[u8]) -> Result<Decoded, ImportError> {
    let mut reader = ImageReader::open(path)?.with_guessed_format()?;
    let format = reader.format().ok_or(ImportError::Unrecognized)?;
    if format == ImageFormat::Jpeg {
        check_jpeg_components(path)?;
    }
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_IMPORT_BYTES);
    reader.limits(limits.clone());

    let mut decoder = reader.into_decoder()?;
    // Everything below is decided from the header, before a single pixel is decoded.
    let (width, height) = decoder.dimensions();
    let color = decoder.color_type();
    let (layout, sample) = layout_of(color)?;
    check_budget(
        width,
        height,
        layout,
        sample,
        layout.channels() * sample.bytes(),
    )?;
    // `into_decoder` bypasses the reader's own accounting of the output buffer.
    limits.reserve(decoder.total_bytes())?;

    let icc = decoder.icc_profile()?;
    let orientation = Orientation::from_image(decoder.orientation()?);
    let mut warnings = Vec::new();
    let space = match format {
        ImageFormat::Png => png_color(path, &mut warnings)?,
        ImageFormat::OpenExr => exr_color(path, &mut warnings)?,
        // QOI header byte 13: 0 = sRGB with linear alpha, 1 = all channels linear.
        ImageFormat::Qoi if head.get(13) == Some(&1) => Some(ColorSpace::LINEAR_SRGB),
        _ => None,
    };
    if is_animated(format, path) {
        warnings.push(ImportWarning::FirstFrameOnly);
    }
    let len = usize::try_from(decoder.total_bytes())
        .map_err(|_| ImportError::TooLarge { width, height })?;
    let mut pixels = vec![0u8; len];
    decoder.read_image(&mut pixels)?;

    Ok(Decoded {
        size: Size::new(width, height),
        layout,
        sample,
        // OpenEXR stores premultiplied (associated) alpha by convention.
        alpha: if format == ImageFormat::OpenExr {
            AlphaMode::Premultiplied
        } else {
            AlphaMode::Straight
        },
        icc,
        space,
        orientation,
        pixels,
        warnings,
    })
}

/// Color a PNG declares besides an embedded ICC profile, with the precedence of the PNG
/// specification (3rd edition): cICP > iCCP > sRGB > gAMA/cHRM. `None`: use the ICC profile
/// if there is one, otherwise the default.
fn png_color(
    path: &Path,
    warnings: &mut Vec<ImportWarning>,
) -> Result<Option<ColorSpace>, ImportError> {
    let reader = png::Decoder::new(BufReader::new(File::open(path)?))
        .read_info()
        .map_err(|e| ImportError::Decode(e.to_string()))?;
    let info = reader.info();
    if let Some(cicp) = &info.coding_independent_code_points {
        match cicp_space(cicp) {
            Some(space) => return Ok(Some(space)),
            None => warnings.push(ImportWarning::ColorInfoUnsupported),
        }
    }
    if info.icc_profile.is_some() {
        return Ok(None);
    }
    if info.srgb.is_some() {
        return Ok(Some(ColorSpace::SRGB));
    }
    if info.gama_chunk.is_none() && info.chrm_chunk.is_none() {
        return Ok(None);
    }
    // gAMA stores the encoding exponent (0.45455 for a 2.2 display gamma).
    let transfer = match info.gama_chunk.map(|g| g.into_value()) {
        None => TransferFunction::Srgb,
        Some(g) if (g - 1.0).abs() < 1e-4 => TransferFunction::Linear,
        Some(g) if g > 0.0 && g.is_finite() => TransferFunction::Gamma(1.0 / g),
        Some(_) => {
            warnings.push(ImportWarning::ColorInfoUnsupported);
            return Ok(None);
        }
    };
    let primaries = match &info.chrm_chunk {
        None => RgbPrimaries::REC709,
        Some(c) => {
            let xy = |(x, y): (png::ScaledFloat, png::ScaledFloat)| {
                [f64::from(x.into_value()), f64::from(y.into_value())]
            };
            let primaries = RgbPrimaries {
                red: xy(c.red),
                green: xy(c.green),
                blue: xy(c.blue),
                white: xy(c.white),
            };
            if !primaries.is_valid() {
                warnings.push(ImportWarning::ColorInfoUnsupported);
                return Ok(None);
            }
            primaries
        }
    };
    Ok(Some(icc::snap(ColorSpace {
        primaries,
        transfer,
    })))
}

/// Primaries an OpenEXR file declares (`chromaticities` attribute), with linear transfer (EXR
/// samples are scene-linear). `None`: the default, linear Rec.709/sRGB primaries, which is also
/// what the OpenEXR specification assumes when the attribute is absent. Reads the headers only.
fn exr_color(
    path: &Path,
    warnings: &mut Vec<ImportWarning>,
) -> Result<Option<ColorSpace>, ImportError> {
    let meta = exr::meta::MetaData::read_from_file(path, false)
        .map_err(|e| ImportError::Decode(e.to_string()))?;
    // The attribute is shared by all the parts of a file (OpenEXR specification).
    let Some(c) = meta
        .headers
        .first()
        .and_then(|h| h.shared_attributes.chromaticities)
    else {
        return Ok(None);
    };
    let xy = |v: exr::math::Vec2<f32>| [f64::from(v.0), f64::from(v.1)];
    let primaries = RgbPrimaries {
        red: xy(c.red),
        green: xy(c.green),
        blue: xy(c.blue),
        white: xy(c.white),
    };
    if !primaries.is_valid() {
        warnings.push(ImportWarning::ColorInfoUnsupported);
        return Ok(None);
    }
    Ok(Some(icc::snap(ColorSpace {
        primaries,
        transfer: TransferFunction::Linear,
    })))
}

/// ITU-T H.273 code points (PNG cICP chunk) as a color space, for RGB full-range data.
fn cicp_space(cicp: &png::CodingIndependentCodePoints) -> Option<ColorSpace> {
    cicp_code_space(
        cicp.color_primaries,
        cicp.transfer_function,
        cicp.matrix_coefficients,
        cicp.is_video_full_range_image,
    )
}

/// ITU-T H.273 code points as a color space, for RGB (matrix 0) full-range data.
pub(crate) fn cicp_code_space(
    primaries: u8,
    transfer: u8,
    matrix: u8,
    full_range: bool,
) -> Option<ColorSpace> {
    if matrix != 0 || !full_range {
        return None;
    }
    let primaries = match primaries {
        1 => RgbPrimaries::REC709,
        9 => RgbPrimaries::REC2020,
        12 => RgbPrimaries::DISPLAY_P3,
        _ => return None,
    };
    let transfer = match transfer {
        1 | 6 | 14 | 15 => TransferFunction::Rec709,
        4 => TransferFunction::Gamma(2.2),
        5 => TransferFunction::Gamma(2.8),
        8 => TransferFunction::Linear,
        13 => TransferFunction::Srgb,
        16 => TransferFunction::Pq,
        18 => TransferFunction::Hlg,
        _ => return None,
    };
    Some(ColorSpace {
        primaries,
        transfer,
    })
}

fn layout_of(color: ColorType) -> Result<(ChannelLayout, SampleType), ImportError> {
    Ok(match color {
        ColorType::L8 => (ChannelLayout::Gray, SampleType::U8),
        ColorType::La8 => (ChannelLayout::GrayAlpha, SampleType::U8),
        ColorType::Rgb8 => (ChannelLayout::Rgb, SampleType::U8),
        ColorType::Rgba8 => (ChannelLayout::Rgba, SampleType::U8),
        ColorType::L16 => (ChannelLayout::Gray, SampleType::U16),
        ColorType::La16 => (ChannelLayout::GrayAlpha, SampleType::U16),
        ColorType::Rgb16 => (ChannelLayout::Rgb, SampleType::U16),
        ColorType::Rgba16 => (ChannelLayout::Rgba, SampleType::U16),
        ColorType::Rgb32F => (ChannelLayout::Rgb, SampleType::F32),
        ColorType::Rgba32F => (ChannelLayout::Rgba, SampleType::F32),
        other => return Err(ImportError::UnsupportedPixels(format!("{other:?}"))),
    })
}

/// Refuse a `width × height` image whose import would exceed [`MAX_IMPORT_BYTES`]: the decoder
/// buffers (`decoded_bytes_per_pixel`), a possible reoriented copy of the same size, and the
/// tiles of a `layout`/`sample` image with their pyramid (padding included: a thin strip needs
/// far more than its pixel count suggests).
pub(crate) fn check_budget(
    width: u32,
    height: u32,
    layout: ChannelLayout,
    sample: SampleType,
    decoded_bytes_per_pixel: u32,
) -> Result<(), ImportError> {
    let too_large = || ImportError::TooLarge { width, height };
    if width == 0 || height == 0 {
        return Err(too_large());
    }
    let size = Size::new(width, height);
    let format = PixelFormat {
        layout,
        sample,
        color_space: ColorSpace::SRGB,
        alpha: AlphaMode::Straight,
    };
    let needed = size
        .pixel_count()
        .checked_mul(u64::from(decoded_bytes_per_pixel) * 2)
        .and_then(|buffers| {
            buffers.checked_add(RasterImage::estimated_memory_bytes(size, format)?)
        });
    match needed {
        Some(bytes) if bytes <= MAX_IMPORT_BYTES => Ok(()),
        _ => Err(too_large()),
    }
}

fn is_tiff(head: &[u8]) -> bool {
    head.starts_with(b"II*\0")
        || head.starts_with(b"MM\0*")
        || head.starts_with(b"II+\0")
        || head.starts_with(b"MM\0+")
}

/// Formats SlopShop recognizes but does not open yet, with the name shown to the user.
fn not_supported_yet(head: &[u8], path: &Path) -> Option<ImportError> {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    // Camera RAW files are often TIFF containers: check the extension before the content.
    const RAW: [&str; 24] = [
        "cr2", "cr3", "crw", "nef", "nrw", "arw", "srf", "sr2", "raf", "orf", "rw2", "rwl", "pef",
        "srw", "dng", "3fr", "fff", "iiq", "x3f", "erf", "kdc", "dcr", "mrw", "mos",
    ];
    if RAW.contains(&extension.as_str()) {
        return Some(ImportError::NotYetSupported("camera RAW"));
    }
    if let Some(error) = heif_brand(head) {
        return Some(error);
    }
    let by_extension = match extension.as_str() {
        "kra" => Some("Krita"),
        "xcf" => Some("GIMP XCF"),
        "ora" => Some("OpenRaster"),
        "eps" | "ai" => Some("EPS/AI"),
        _ => None,
    };
    by_extension.map(ImportError::NotYetSupported)
}

/// HEIF containers (ISO BMFF `ftyp` box): AVIF is planned, HEIC is refused (ADR 0006). The
/// codec is told by the major and compatible brands; generic brands alone count as HEIC.
fn heif_brand(head: &[u8]) -> Option<ImportError> {
    if head.len() < 12 || &head[4..8] != b"ftyp" {
        return None;
    }
    let box_size = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
    let end = box_size.clamp(16, head.len().max(16)).min(head.len());
    // Major brand, then compatible brands after the minor version.
    let brands: Vec<&[u8]> = std::iter::once(&head[8..12])
        .chain(
            head.get(16..end)
                .unwrap_or_default()
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| b.as_slice()),
        )
        .collect();
    let any = |names: &[&[u8; 4]]| brands.iter().any(|b| names.iter().any(|n| *b == *n));
    // AVIF is read (`avif`); it also names the generic HEIF brands.
    if any(&[b"avif", b"avis"]) {
        return None;
    }
    let heic = [
        b"heic", b"heix", b"hevc", b"hevx", b"heim", b"heis", b"hevm", b"hevs",
    ];
    if any(&heic)
        || [b"mif1", b"msf1", b"miaf"]
            .iter()
            .any(|g| &head[8..12] == *g)
    {
        return Some(ImportError::HeicUnsupported);
    }
    None
}

/// Refuse JPEGs whose components are not gray or RGB/YCbCr (CMYK, YCCK), which the decoder
/// would otherwise convert to RGB silently. The header is parsed by the decoder `image` uses,
/// with the same options, so that both see the same frame header; a header it cannot parse is
/// an error, never a pass.
fn check_jpeg_components(path: &Path) -> Result<(), ImportError> {
    use zune_jpeg::zune_core::colorspace::ColorSpace as JpegColor;
    use zune_jpeg::zune_core::options::DecoderOptions;
    let options = DecoderOptions::default()
        .set_strict_mode(false)
        .set_max_width(usize::MAX)
        .set_max_height(usize::MAX);
    let mut decoder =
        zune_jpeg::JpegDecoder::new_with_options(BufReader::new(File::open(path)?), options);
    decoder
        .decode_headers()
        .map_err(|e| ImportError::Decode(format!("{e:?}")))?;
    match decoder.input_colorspace() {
        Some(
            JpegColor::RGB
            | JpegColor::RGBA
            | JpegColor::YCbCr
            | JpegColor::Luma
            | JpegColor::LumaA,
        ) => Ok(()),
        Some(JpegColor::CMYK | JpegColor::YCCK) => {
            Err(ImportError::UnsupportedPixels("CMYK JPEG".into()))
        }
        other => Err(ImportError::UnsupportedPixels(format!("JPEG {other:?}"))),
    }
}

/// Whether an animated container holds more than one frame (only the first is imported).
fn is_animated(format: ImageFormat, path: &Path) -> bool {
    let Ok(file) = File::open(path) else {
        return false;
    };
    match format {
        ImageFormat::Gif => {
            // Frame headers only: no LZW decoding, no canvas.
            let mut options = gif::DecodeOptions::new();
            options.skip_frame_decoding(true);
            match options.read_info(BufReader::new(file)) {
                Ok(mut decoder) => (0..2).all(|_| matches!(decoder.next_frame_info(), Ok(Some(_)))),
                Err(_) => false,
            }
        }
        ImageFormat::Png => image::codecs::png::PngDecoder::new(BufReader::new(file))
            .and_then(|d| d.is_apng())
            .unwrap_or(false),
        ImageFormat::WebP => image::codecs::webp::WebPDecoder::new(BufReader::new(file))
            .map(|d| d.has_animation())
            .unwrap_or(false),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;
    use slopshop_core::color::TransferFunction;
    use slopshop_core::raster::TILE_SIZE;
    use slopshop_core::tile::TileCoord;

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("slopshop-io-{}-{name}", std::process::id()))
    }

    /// Write `bytes` to a temporary file, open it, and clean up.
    fn open_bytes(name: &str, bytes: &[u8]) -> Result<Imported, ImportError> {
        let path = temp_path(name);
        std::fs::write(&path, bytes).unwrap();
        let result = open_image(&path);
        std::fs::remove_file(&path).ok();
        result
    }

    fn encode(format: ImageFormat, image: impl Into<image::DynamicImage>) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        image.into().write_to(&mut out, format).unwrap();
        out.into_inner()
    }

    /// Stored bytes of pixel (x, y) of level 0.
    fn stored_pixel(img: &RasterImage, x: u32, y: u32) -> Vec<u8> {
        let bpp = img.stored_format().bytes_per_pixel() as usize;
        let tile = img.levels()[0]
            .tile(TileCoord {
                col: x / TILE_SIZE,
                row: y / TILE_SIZE,
            })
            .unwrap();
        let i = ((y % TILE_SIZE) * TILE_SIZE + x % TILE_SIZE) as usize * bpp;
        tile[i..i + bpp].to_vec()
    }

    #[test]
    fn png_8_bit_keeps_pixels() {
        let mut img = image::RgbImage::new(3, 2);
        img.put_pixel(0, 0, image::Rgb([10, 20, 30]));
        img.put_pixel(2, 1, image::Rgb([200, 100, 50]));
        let imported = open_bytes("rgb.png", &encode(ImageFormat::Png, img)).unwrap();
        let image = imported.image;
        assert_eq!(image.size(), Size::new(3, 2));
        assert_eq!(image.format().color_space, ColorSpace::SRGB);
        assert_eq!(stored_pixel(&image, 0, 0), [10, 20, 30, 255]);
        assert_eq!(stored_pixel(&image, 2, 1), [200, 100, 50, 255]);
        assert!(imported.warnings.is_empty());
    }

    #[test]
    fn png_16_bit_is_kept_at_16_bit() {
        let mut img = image::ImageBuffer::<image::Luma<u16>, _>::new(2, 1);
        img.put_pixel(0, 0, image::Luma([1]));
        img.put_pixel(1, 0, image::Luma([65000]));
        let imported = open_bytes("gray16.png", &encode(ImageFormat::Png, img)).unwrap();
        let format = imported.image.format();
        assert_eq!(
            (format.layout, format.sample),
            (ChannelLayout::Gray, SampleType::U16)
        );
        assert_eq!(stored_pixel(&imported.image, 0, 0), 1u16.to_ne_bytes());
        assert_eq!(stored_pixel(&imported.image, 1, 0), 65000u16.to_ne_bytes());
    }

    #[test]
    fn embedded_icc_profile_is_applied() {
        let icc = icc::write_matrix_trc(&ColorSpace::DISPLAY_P3).unwrap();
        let mut out = std::io::Cursor::new(Vec::new());
        let mut encoder = image::codecs::png::PngEncoder::new(&mut out);
        encoder.set_icc_profile(icc).unwrap();
        encoder
            .write_image(&[255, 0, 0], 1, 1, image::ExtendedColorType::Rgb8)
            .unwrap();
        let imported = open_bytes("p3.png", &out.into_inner()).unwrap();
        assert_eq!(imported.image.format().color_space, ColorSpace::DISPLAY_P3);
        assert!(imported.warnings.is_empty());
    }

    #[test]
    fn float_tiff_is_kept_exactly() {
        let path = temp_path("f32.tif");
        {
            let file = File::create(&path).unwrap();
            let mut tiff = tiff::encoder::TiffEncoder::new(file).unwrap();
            let data = [2.5f32, -0.25, 1e-6, 0.0, 100.0, 0.5];
            tiff.write_image::<tiff::encoder::colortype::RGB32Float>(2, 1, &data)
                .unwrap();
        }
        let imported = open_image(&path).unwrap();
        std::fs::remove_file(&path).ok();
        let image = imported.image;
        assert_eq!(image.format().sample, SampleType::F32);
        assert_eq!(image.format().color_space, ColorSpace::LINEAR_SRGB);
        let px: Vec<f32> = stored_pixel(&image, 0, 0)
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_ne_bytes(*b))
            .collect();
        assert_eq!(px, [2.5, -0.25, 1e-6, 1.0]);
    }

    #[test]
    fn sixteen_bit_tiff_and_multi_page_warning() {
        let path = temp_path("pages.tif");
        {
            let file = File::create(&path).unwrap();
            let mut tiff = tiff::encoder::TiffEncoder::new(file).unwrap();
            for page in 0..2u16 {
                let data = [page, 40000, 65535, 7];
                tiff.write_image::<tiff::encoder::colortype::Gray16>(2, 2, &data)
                    .unwrap();
            }
        }
        let imported = open_image(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(imported.image.format().sample, SampleType::U16);
        assert_eq!(stored_pixel(&imported.image, 1, 0), 40000u16.to_ne_bytes());
        assert_eq!(imported.warnings, [ImportWarning::FirstPageOnly]);
    }

    #[test]
    fn exr_is_linear_float_with_premultiplied_alpha() {
        let img = image::Rgba32FImage::from_raw(1, 1, vec![4.0, 0.5, 0.25, 1.0]).unwrap();
        let imported = open_bytes("hdr.exr", &encode(ImageFormat::OpenExr, img)).unwrap();
        let format = imported.image.format();
        assert_eq!(format.sample, SampleType::F32);
        assert_eq!(format.alpha, AlphaMode::Premultiplied);
        assert!(format.color_space.transfer.is_linear());
        let px: Vec<f32> = stored_pixel(&imported.image, 0, 0)
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_ne_bytes(*b))
            .collect();
        assert_eq!(px, [4.0, 0.5, 0.25, 1.0], "HDR value above 1 survives");
    }

    /// A small RGBA EXR file written by the `exr` crate, with an optional `chromaticities`
    /// attribute given as (red, green, blue, white) xy.
    fn exr_with_chromaticities(xy: Option<[[f32; 2]; 4]>) -> Vec<u8> {
        use exr::prelude::*;
        let pixels = SpecificChannels::rgba(|Vec2(x, _y): Vec2<usize>| {
            (x as f32 * 2.0, 0.5f32, 0.25f32, 1.0f32)
        });
        let mut image = Image::from_channels((2, 1), pixels);
        image.attributes.chromaticities = xy.map(|[r, g, b, w]| {
            let v = |[x, y]: [f32; 2]| Vec2(x, y);
            exr::meta::attribute::Chromaticities {
                red: v(r),
                green: v(g),
                blue: v(b),
                white: v(w),
            }
        });
        let mut out = std::io::Cursor::new(Vec::new());
        image.write().to_buffered(&mut out).unwrap();
        out.into_inner()
    }

    #[test]
    fn exr_chromaticities_give_the_primaries() {
        let rec709 = [[0.64, 0.33], [0.30, 0.60], [0.15, 0.06], [0.3127, 0.3290]];
        let rec2020 = [
            [0.708, 0.292],
            [0.170, 0.797],
            [0.131, 0.046],
            [0.3127, 0.3290],
        ];
        let p3 = [
            [0.680, 0.320],
            [0.265, 0.690],
            [0.150, 0.060],
            [0.3127, 0.3290],
        ];
        let cases = [
            (Some(rec2020), ColorSpace::LINEAR_REC2020),
            (Some(rec709), ColorSpace::LINEAR_SRGB),
            (
                Some(p3),
                ColorSpace {
                    primaries: RgbPrimaries::DISPLAY_P3,
                    transfer: TransferFunction::Linear,
                },
            ),
            // Absent: Rec.709 primaries, as the OpenEXR specification says.
            (None, ColorSpace::LINEAR_SRGB),
        ];
        for (xy, expected) in cases {
            let imported = open_bytes("chroma.exr", &exr_with_chromaticities(xy)).unwrap();
            let format = imported.image.format();
            let space = format.color_space;
            assert_eq!(space.transfer, TransferFunction::Linear);
            for (got, want) in [
                (space.primaries.red, expected.primaries.red),
                (space.primaries.green, expected.primaries.green),
                (space.primaries.blue, expected.primaries.blue),
                (space.primaries.white, expected.primaries.white),
            ] {
                assert!((got[0] - want[0]).abs() < 1e-6 && (got[1] - want[1]).abs() < 1e-6);
            }
            if expected.id().is_some() {
                assert_eq!(space, expected, "named spaces are snapped exactly");
            }
            assert!(imported.warnings.is_empty());
            // Only the header is read differently: pixels are unchanged.
            assert_eq!(format.alpha, AlphaMode::Premultiplied);
            let px: Vec<f32> = stored_pixel(&imported.image, 1, 0)
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_ne_bytes(*b))
                .collect();
            assert_eq!(px, [2.0, 0.5, 0.25, 1.0]);
        }
    }

    #[test]
    fn degenerate_exr_chromaticities_warn_and_keep_the_default() {
        let coincident = [[0.3, 0.3], [0.3, 0.3], [0.3, 0.3], [0.3127, 0.3290]];
        let imported =
            open_bytes("degenerate.exr", &exr_with_chromaticities(Some(coincident))).unwrap();
        assert_eq!(imported.image.format().color_space, ColorSpace::LINEAR_SRGB);
        assert_eq!(imported.warnings, [ImportWarning::ColorInfoUnsupported]);
    }

    #[test]
    fn radiance_hdr_is_linear_float() {
        let img = image::Rgb32FImage::from_raw(1, 1, vec![8.0, 1.0, 0.5]).unwrap();
        let imported = open_bytes("sky.hdr", &encode(ImageFormat::Hdr, img)).unwrap();
        let format = imported.image.format();
        assert_eq!(format.sample, SampleType::F32);
        assert_eq!(format.color_space.transfer, TransferFunction::Linear);
        let px = stored_pixel(&imported.image, 0, 0);
        let r = f32::from_ne_bytes(px[0..4].try_into().unwrap());
        assert!((r - 8.0).abs() < 0.1, "RGBE keeps HDR values: {r}");
    }

    #[test]
    fn common_8_bit_formats_open() {
        let img = image::RgbaImage::from_pixel(3, 3, image::Rgba([12, 34, 56, 255]));
        for format in [
            ImageFormat::Jpeg,
            ImageFormat::WebP,
            ImageFormat::Bmp,
            ImageFormat::Tga,
            ImageFormat::Qoi,
            ImageFormat::Pnm,
            ImageFormat::Farbfeld,
        ] {
            let dynamic = image::DynamicImage::ImageRgba8(img.clone());
            let bytes = match format {
                ImageFormat::Jpeg | ImageFormat::Pnm => encode(format, dynamic.to_rgb8()),
                // farbfeld is always 16-bit RGBA.
                ImageFormat::Farbfeld => encode(format, dynamic.to_rgba16()),
                _ => encode(format, dynamic),
            };
            let name = format!("common.{}", format.extensions_str()[0]);
            let imported = open_bytes(&name, &bytes).unwrap_or_else(|e| panic!("{format:?}: {e}"));
            assert_eq!(imported.image.size(), Size::new(3, 3), "{format:?}");
        }
    }

    #[test]
    fn animated_gif_warns_first_frame_only() {
        let mut out = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut out);
            let frame = |v| {
                image::Frame::new(image::RgbaImage::from_pixel(
                    2,
                    2,
                    image::Rgba([v, v, v, 255]),
                ))
            };
            encoder.encode_frames([frame(10), frame(200)]).unwrap();
        }
        let imported = open_bytes("anim.gif", &out).unwrap();
        assert_eq!(imported.warnings, [ImportWarning::FirstFrameOnly]);
    }

    #[test]
    fn recognized_formats_that_are_not_supported_yet() {
        let cases: [(&str, Vec<u8>, &str); 6] = [
            ("photo.heic", b"\0\0\0\x18ftypheic\0\0\0\0".to_vec(), "heic"),
            ("logo.eps", b"%!PS-Adobe".to_vec(), "notYetSupported"),
            ("art.xcf", b"gimp xcf ".to_vec(), "notYetSupported"),
            // A CMYK Photoshop document (header only): refused until the engine has CMYK.
            (
                "print.psd",
                [
                    &b"8BPS\0\x01"[..],
                    &[0; 6],
                    &[0, 4, 0, 0, 0, 1, 0, 0, 0, 1, 0, 8, 0, 4],
                ]
                .concat(),
                "notYetSupported",
            ),
            ("shot.cr2", b"II*\0\x10\0\0\0CR".to_vec(), "notYetSupported"),
            ("art.kra", b"PK".to_vec(), "notYetSupported"),
        ];
        for (name, bytes, code) in cases {
            let error = open_bytes(name, &bytes).unwrap_err();
            assert_eq!(error.code(), code, "{name}: {error:?}");
        }
    }

    /// JPEG headers (no scan data) with 4 components, preceded by `before_frame`.
    fn cmyk_jpeg(before_frame: &[u8]) -> Vec<u8> {
        let mut jpeg = vec![0xFF, 0xD8];
        jpeg.extend(before_frame);
        // Baseline frame header, 16 × 16, four components.
        jpeg.extend([
            0xFF, 0xC0, 0x00, 0x14, 0x08, 0x00, 0x10, 0x00, 0x10, 0x04, 1, 0x11, 0, 2, 0x11, 0, 3,
            0x11, 0, 4, 0x11, 0,
        ]);
        // Start of scan over the four components, then end of image.
        jpeg.extend([
            0xFF, 0xDA, 0x00, 0x0E, 0x04, 1, 0x00, 2, 0x00, 3, 0x00, 4, 0x00, 0, 63, 0,
        ]);
        jpeg.extend([0xFF, 0xD9]);
        jpeg
    }

    #[test]
    fn cmyk_jpeg_is_refused_instead_of_converted() {
        // An APP1 segment of 65 533 bytes.
        let app1: Vec<u8> = [0xFF, 0xE1, 0xFF, 0xFF]
            .into_iter()
            .chain(std::iter::repeat_n(0, 65533))
            .collect();
        let cases = [
            ("plain", cmyk_jpeg(&[])),
            // A stray byte before the frame header.
            ("junk", cmyk_jpeg(&[0x00])),
            // More than 4 MiB of metadata before the frame header.
            ("metadata", cmyk_jpeg(&app1.repeat(70))),
        ];
        for (name, bytes) in cases {
            let error = open_bytes(&format!("cmyk-{name}.jpg"), &bytes).unwrap_err();
            assert_eq!(error.code(), "unsupportedPixels", "{name}: {error:?}");
        }
        // Without a readable frame header: an error, never a pass.
        let error = open_bytes("broken.jpg", &[0xFF, 0xD8, 0xFF, 0xD9]).unwrap_err();
        assert_eq!(error.code(), "decode", "{error:?}");
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &b in bytes {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend((data.len() as u32).to_be_bytes());
        let start = out.len();
        out.extend(kind);
        out.extend(data);
        let crc = crc32(&out[start..]);
        out.extend(crc.to_be_bytes());
    }

    /// A 1 × 1 RGB PNG with extra chunks right after IHDR.
    fn png_with_chunks(chunks: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let png = encode(ImageFormat::Png, image::RgbImage::new(1, 1));
        // Signature (8) + IHDR (4 + 4 + 13 + 4).
        let mut out = png[..33].to_vec();
        for (kind, data) in chunks {
            chunk(&mut out, kind, data);
        }
        out.extend(&png[33..]);
        out
    }

    #[test]
    fn png_color_chunks_are_honored_in_order() {
        let gamma = |g: u32| g.to_be_bytes();
        // Name, chunks after IHDR, expected space, whether a warning is expected.
        type Case<'a> = (&'a str, Vec<(&'a [u8; 4], Vec<u8>)>, ColorSpace, bool);
        let cases: [Case; 5] = [
            (
                "gamma-linear",
                vec![(b"gAMA", gamma(100_000).to_vec())],
                ColorSpace::LINEAR_SRGB,
                false,
            ),
            (
                "srgb-over-gamma",
                vec![(b"sRGB", vec![0]), (b"gAMA", gamma(45_455).to_vec())],
                ColorSpace::SRGB,
                false,
            ),
            (
                "cicp-pq",
                vec![(b"cICP", vec![9, 16, 0, 1])],
                ColorSpace::REC2100_PQ,
                false,
            ),
            (
                "cicp-over-srgb",
                vec![(b"cICP", vec![12, 13, 0, 1]), (b"sRGB", vec![0])],
                ColorSpace::DISPLAY_P3,
                false,
            ),
            // BT.601 primaries: not representable yet, reported.
            (
                "cicp-unsupported",
                vec![(b"cICP", vec![5, 13, 0, 1])],
                ColorSpace::SRGB,
                true,
            ),
        ];
        for (name, chunks, expected, warned) in cases {
            let chunks: Vec<(&[u8; 4], &[u8])> =
                chunks.iter().map(|(k, d)| (*k, d.as_slice())).collect();
            let imported = open_bytes(&format!("{name}.png"), &png_with_chunks(&chunks))
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(imported.image.format().color_space, expected, "{name}");
            assert_eq!(
                imported
                    .warnings
                    .contains(&ImportWarning::ColorInfoUnsupported),
                warned,
                "{name}"
            );
        }
        // gAMA 1/2.2 alone is a 2.2 power law, not the sRGB curve.
        let imported = open_bytes(
            "gamma22.png",
            &png_with_chunks(&[(b"gAMA", &gamma(45_455))]),
        )
        .unwrap();
        let TransferFunction::Gamma(g) = imported.image.format().color_space.transfer else {
            panic!("{:?}", imported.image.format().color_space);
        };
        assert!((g - 2.2).abs() < 1e-3, "{g}");
    }

    #[test]
    fn linear_qoi_is_linear() {
        let mut qoi = encode(ImageFormat::Qoi, image::RgbImage::new(2, 2));
        assert_eq!(qoi[13], 0);
        let imported = open_bytes("srgb.qoi", &qoi).unwrap();
        assert_eq!(imported.image.format().color_space, ColorSpace::SRGB);
        qoi[13] = 1;
        let imported = open_bytes("linear.qoi", &qoi).unwrap();
        assert_eq!(imported.image.format().color_space, ColorSpace::LINEAR_SRGB);
    }

    #[test]
    fn float_images_with_infinities_are_reported() {
        let img = image::Rgba32FImage::from_raw(
            2,
            1,
            vec![f32::INFINITY, 0.5, 0.25, 1.0, 0.0, 0.0, 0.0, 1.0],
        )
        .unwrap();
        let imported = open_bytes("inf.exr", &encode(ImageFormat::OpenExr, img)).unwrap();
        assert_eq!(imported.warnings, [ImportWarning::NonFiniteSamples]);
    }

    #[test]
    fn heif_brands_tell_avif_from_heic() {
        let ftyp = |major: &[u8; 4], compatible: &[&[u8; 4]]| {
            let mut v = Vec::new();
            v.extend(((16 + 4 * compatible.len()) as u32).to_be_bytes());
            v.extend(b"ftyp");
            v.extend(major);
            v.extend([0, 0, 0, 0]);
            for b in compatible {
                v.extend(*b);
            }
            v
        };
        assert!(heif_brand(&ftyp(b"mif1", &[b"mif1", b"avif", b"miaf"])).is_none());
        assert!(heif_brand(&ftyp(b"avis", &[])).is_none());
        let cases = [
            (ftyp(b"hevs", &[]), "heic"),
            (ftyp(b"mif1", &[b"mif1", b"heic"]), "heic"),
            (ftyp(b"msf1", &[b"hevm"]), "heic"),
        ];
        for (bytes, code) in cases {
            assert_eq!(
                heif_brand(&bytes).map(|e| e.code()),
                Some(code),
                "{bytes:?}"
            );
        }
        // Other ISO BMFF files (MP4 video) are not HEIF.
        assert!(heif_brand(&ftyp(b"isom", &[b"mp41"])).is_none());
    }

    #[test]
    fn single_frame_gif_has_no_warning() {
        let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255]));
        let imported = open_bytes("still.gif", &encode(ImageFormat::Gif, img)).unwrap();
        assert!(imported.warnings.is_empty(), "{:?}", imported.warnings);
    }

    /// TIFF sample layouts the `tiff` encoder has no type for.
    macro_rules! tiff_color {
        ($name:ident, $photometric:ident, $bits:expr, $formats:expr) => {
            struct $name;
            impl tiff::encoder::colortype::ColorType for $name {
                type Inner = u8;
                const TIFF_VALUE: tiff::tags::PhotometricInterpretation =
                    tiff::tags::PhotometricInterpretation::$photometric;
                const BITS_PER_SAMPLE: &'static [u16] = $bits;
                const SAMPLE_FORMAT: &'static [tiff::tags::SampleFormat] = $formats;
                fn horizontal_predict(row: &[u8], result: &mut Vec<u8>) {
                    result.extend_from_slice(row);
                }
            }
        };
    }
    const UINT: tiff::tags::SampleFormat = tiff::tags::SampleFormat::Uint;
    tiff_color!(GrayAlpha8, BlackIsZero, &[8, 8], &[UINT, UINT]);
    tiff_color!(Bilevel, BlackIsZero, &[1], &[UINT]);

    /// Write a one-page TIFF with extra tags, and open it.
    fn open_tiff<C: tiff::encoder::colortype::ColorType<Inner = u8>>(
        name: &str,
        size: (u32, u32),
        tags: &[(tiff::tags::Tag, u16)],
        data: &[u8],
    ) -> Result<Imported, ImportError> {
        let path = temp_path(name);
        {
            let file = File::create(&path).unwrap();
            let mut tiff = tiff::encoder::TiffEncoder::new(file).unwrap();
            let mut image = tiff.new_image::<C>(size.0, size.1).unwrap();
            for (tag, value) in tags {
                image.encoder().write_tag(*tag, *value).unwrap();
            }
            image.write_data(data).unwrap();
        }
        let result = open_image(&path);
        std::fs::remove_file(&path).ok();
        result
    }

    #[test]
    fn gray_alpha_tiff_opens_with_its_alpha_mode() {
        use tiff::tags::Tag;
        let data = [10, 255, 200, 128];
        let straight =
            open_tiff::<GrayAlpha8>("ga.tif", (2, 1), &[(Tag::ExtraSamples, 2)], &data).unwrap();
        let format = straight.image.format();
        assert_eq!(
            (format.layout, format.alpha),
            (ChannelLayout::GrayAlpha, AlphaMode::Straight)
        );
        assert_eq!(stored_pixel(&straight.image, 1, 0), [200, 128]);
        let associated =
            open_tiff::<GrayAlpha8>("gpa.tif", (2, 1), &[(Tag::ExtraSamples, 1)], &data).unwrap();
        assert_eq!(associated.image.format().alpha, AlphaMode::Premultiplied);
        // Unspecified extra sample: not alpha, refused rather than guessed.
        let error = open_tiff::<GrayAlpha8>("gx.tif", (2, 1), &[(Tag::ExtraSamples, 0)], &data)
            .unwrap_err();
        assert_eq!(error.code(), "unsupportedPixels", "{error:?}");
    }

    /// A little-endian 2 × 1 RGB 8-bit TIFF with PlanarConfiguration = 2.
    fn planar_rgb_tiff() -> Vec<u8> {
        const SHORT: u16 = 3;
        const LONG: u16 = 4;
        // Header (8), then 10 entries (2 + 10 × 12 + 4 = 126): extra data from 134.
        let (bits_at, offsets_at, counts_at, pixels_at) = (134u32, 140u32, 152u32, 158u32);
        let entries: [(u16, u16, u32, u32); 10] = [
            (256, SHORT, 1, 2),         // ImageWidth
            (257, SHORT, 1, 1),         // ImageLength
            (258, SHORT, 3, bits_at),   // BitsPerSample
            (259, SHORT, 1, 1),         // Compression: none
            (262, SHORT, 1, 2),         // PhotometricInterpretation: RGB
            (273, LONG, 3, offsets_at), // StripOffsets
            (277, SHORT, 1, 3),         // SamplesPerPixel
            (278, SHORT, 1, 1),         // RowsPerStrip
            (279, SHORT, 3, counts_at), // StripByteCounts
            (284, SHORT, 1, 2),         // PlanarConfiguration: planar
        ];
        let mut v = b"II*\0".to_vec();
        v.extend(8u32.to_le_bytes());
        v.extend((entries.len() as u16).to_le_bytes());
        for (tag, kind, count, value) in entries {
            v.extend(tag.to_le_bytes());
            v.extend(kind.to_le_bytes());
            v.extend(count.to_le_bytes());
            if kind == SHORT && count == 1 {
                v.extend((value as u16).to_le_bytes());
                v.extend([0, 0]);
            } else {
                v.extend(value.to_le_bytes());
            }
        }
        v.extend(0u32.to_le_bytes()); // no next IFD
        assert_eq!(v.len() as u32, bits_at);
        for _ in 0..3 {
            v.extend(8u16.to_le_bytes());
        }
        for plane in 0..3 {
            v.extend((pixels_at + plane * 2).to_le_bytes());
        }
        for _ in 0..3 {
            v.extend(2u16.to_le_bytes());
        }
        assert_eq!(v.len() as u32, pixels_at);
        v.extend([255, 0, 0, 255, 0, 0]); // R plane, G plane, B plane
        v
    }

    #[test]
    fn sub_byte_and_planar_tiffs_are_refused_cleanly() {
        use tiff::tags::Tag;
        // 1-bit scan with an orientation tag (used to panic in orientation).
        let error =
            open_tiff::<Bilevel>("bw.tif", (8, 8), &[(Tag::Orientation, 6)], &[0; 64]).unwrap_err();
        assert_eq!(error.code(), "unsupportedPixels", "{error:?}");
        // Planar RGB, 2 × 1: one strip per plane (the encoder cannot write this).
        let error = open_bytes("planar.tif", &planar_rgb_tiff()).unwrap_err();
        assert_eq!(error.code(), "unsupportedPixels", "{error:?}");
    }

    #[test]
    fn decoder_output_of_the_wrong_length_is_an_error() {
        let decoded = Decoded {
            size: Size::new(4, 4),
            layout: ChannelLayout::Gray,
            sample: SampleType::U8,
            alpha: AlphaMode::Straight,
            icc: None,
            space: None,
            orientation: Orientation::Rotate90,
            pixels: vec![0; 2],
            warnings: Vec::new(),
        };
        assert_eq!(finish(decoded).unwrap_err().code(), "decode");
    }

    /// A valid PNG whose header claims a huge size, without the pixel data.
    fn png_header_only(width: u32, height: u32) -> Vec<u8> {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend(width.to_be_bytes());
        ihdr.extend(height.to_be_bytes());
        ihdr.extend([8, 6, 0, 0, 0]); // 8-bit RGBA, no interlace
        chunk(&mut png, b"IHDR", &ihdr);
        chunk(&mut png, b"IDAT", &[]);
        chunk(&mut png, b"IEND", &[]);
        png
    }

    #[test]
    fn crafted_header_is_rejected_before_allocating() {
        // Would need ~250 GB: must be an error, not an allocation-failure abort.
        let result = open_bytes("bomb.png", &png_header_only(250_000, 250_000));
        assert!(
            matches!(
                result,
                Err(ImportError::TooLarge {
                    width: 250_000,
                    height: 250_000
                })
            ),
            "{result:?}"
        );
    }

    #[test]
    fn budget_accepts_large_real_images() {
        // The 233 MP NASA test image, a 100 MP camera file, and a 16-bit 50 MP scan.
        use ChannelLayout::{Rgb, Rgba};
        use SampleType::{U8, U16};
        assert!(check_budget(21_600, 10_800, Rgb, U8, 3).is_ok());
        assert!(check_budget(12_000, 8_400, Rgba, U8, 4).is_ok());
        assert!(check_budget(8_660, 5_774, Rgba, U16, 8).is_ok());
        assert!(check_budget(u32::MAX, u32::MAX, Rgb, U8, 3).is_err());
        // A thin strip: few pixels, but ~256× more in padded tiles.
        assert!(matches!(
            check_budget(1, 50_000_000, Rgba, U8, 4),
            Err(ImportError::TooLarge { .. })
        ));
    }

    /// An 8-bit PNG of `color`, one row of `data`, with a `tRNS` chunk.
    fn png_with_trns(color: ::png::ColorType, palette: &[u8], trns: &[u8], data: &[u8]) -> Vec<u8> {
        let width = (data.len() / color.samples()) as u32;
        let mut out = Vec::new();
        let mut encoder = ::png::Encoder::new(&mut out, width, 1);
        encoder.set_color(color);
        encoder.set_depth(::png::BitDepth::Eight);
        if !palette.is_empty() {
            encoder.set_palette(palette.to_vec());
        }
        encoder.set_trns(trns.to_vec());
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(data).unwrap();
        writer.finish().unwrap();
        out
    }

    #[test]
    fn png_trns_transparency_becomes_alpha() {
        // Palette with partial alpha: red transparent, green half, blue opaque (no entry).
        let palette = png_with_trns(
            ::png::ColorType::Indexed,
            &[255, 0, 0, 0, 255, 0, 0, 0, 255],
            &[0, 128],
            &[0, 1, 2],
        );
        let image = open_bytes("palette-trns.png", &palette).unwrap().image;
        assert!(image.format().layout.has_alpha());
        assert_eq!(stored_pixel(&image, 0, 0)[3], 0);
        assert_eq!(stored_pixel(&image, 1, 0), [0, 255, 0, 128]);
        assert_eq!(stored_pixel(&image, 2, 0), [0, 0, 255, 255]);

        // RGB with one transparent color (16-bit sample values in tRNS).
        let rgb = png_with_trns(
            ::png::ColorType::Rgb,
            &[],
            &[0, 10, 0, 20, 0, 30],
            &[10, 20, 30, 1, 2, 3],
        );
        let image = open_bytes("rgb-trns.png", &rgb).unwrap().image;
        assert!(image.format().layout.has_alpha());
        assert_eq!(stored_pixel(&image, 0, 0)[3], 0);
        assert_eq!(stored_pixel(&image, 1, 0), [1, 2, 3, 255]);

        // Gray with one transparent level.
        let gray = png_with_trns(::png::ColorType::Grayscale, &[], &[0, 7], &[7, 200]);
        let image = open_bytes("gray-trns.png", &gray).unwrap().image;
        let format = image.format();
        assert!(format.layout.has_alpha(), "{format:?}");
        let (transparent, opaque) = (stored_pixel(&image, 0, 0), stored_pixel(&image, 1, 0));
        assert_eq!(transparent.last(), Some(&0));
        assert_eq!(opaque.last(), Some(&255));
    }

    #[test]
    fn missing_file_is_an_io_error() {
        assert!(matches!(
            open_image(Path::new("does/not/exist.png")),
            Err(ImportError::Io(_))
        ));
    }
}
