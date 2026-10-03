//! FITS import (in-house: 2880-byte blocks of 80-character header cards, big-endian data): the
//! first image (the primary HDU, else the first IMAGE extension), shown through an automatic
//! stretch as a Levels adjustment layer above it, as astronomy software does (the maintainer's
//! choice: raw values plus an adjustment, see formats.md).
//!
//! Samples: BITPIX 8 as 8-bit, 16 as 16-bit (signed values offset by half the range: BZERO
//! 32768, the unsigned convention, gives the same bits), 32 and 64 as 32-bit float over their
//! whole range (reported: less precise), −32 and −64 as 32-bit float scaled from their own
//! range to [0, 1] (reported); blank (NaN) samples stay NaN. A third axis of 3 is RGB; other
//! cubes give their first plane (reported). FITS stores the bottom row first: it is flipped.
//! Tile-compressed images (in binary tables) are not supported yet.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use slopshop_core::Size;
use slopshop_core::adjust::Adjustment;
use slopshop_core::color::{AlphaMode, ChannelLayout, ColorSpace, SampleType};

use crate::orient::Orientation;
use crate::{Decoded, ImportError, ImportWarning, Opened, adjusted, check_budget, finish};

const BLOCK: u64 = 2880;
const CARD: usize = 80;
/// Bounds on what a header may declare, before anything is allocated.
const MAX_HEADER_BLOCKS: u64 = 10_000;
const MAX_HDUS: usize = 1000;

/// Whether `head` starts like a FITS file.
pub(crate) fn is_fits(head: &[u8]) -> bool {
    head.starts_with(b"SIMPLE  =")
}

/// The image and its stretch as Levels, in an isolated group.
pub(crate) fn open(path: &Path) -> Result<Opened, ImportError> {
    let (decoded, levels) = read(path)?;
    let imported = finish(decoded)?;
    let name = path
        .file_stem()
        .map_or_else(|| "FITS".to_owned(), |s| s.to_string_lossy().into_owned());
    match levels {
        // An isolated group: the stretch changes this image only.
        Some(levels) => adjusted::layered(
            name.clone(),
            vec![(name, imported)],
            Some(("STF auto".to_owned(), levels)),
        ),
        None => Ok(Opened::Image(imported)),
    }
}

/// The image as shown: the stretch applied (reported as a flattened document).
pub(crate) fn decode(path: &Path) -> Result<Decoded, ImportError> {
    let (mut decoded, levels) = read(path)?;
    if let Some(levels) = levels {
        adjusted::apply_levels(&levels, &mut decoded);
        decoded.warnings.push(ImportWarning::LayersFlattened);
    }
    Ok(decoded)
}

fn corrupt(what: &str) -> ImportError {
    ImportError::Decode(format!("FITS: {what}"))
}

/// The cards of one header: keyword and value text (comments and quotes removed).
struct Header {
    cards: Vec<(String, String)>,
}

impl Header {
    fn get(&self, key: &str) -> Option<&str> {
        self.cards
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    fn int(&self, key: &str) -> Option<i64> {
        self.get(key)?.parse().ok()
    }

    fn float(&self, key: &str) -> Option<f64> {
        // Fortran writes exponents with D.
        self.get(key)?.replace(['D', 'd'], "E").parse().ok()
    }

    fn flag(&self, key: &str) -> bool {
        self.get(key) == Some("T")
    }
}

/// One header's cards, up to END; the reader is left at the start of its data.
fn read_header(input: &mut impl Read) -> Result<Option<Header>, ImportError> {
    let mut cards = Vec::new();
    for block in 0.. {
        if block >= MAX_HEADER_BLOCKS {
            return Err(corrupt("header too long"));
        }
        let mut bytes = [0u8; BLOCK as usize];
        match input.read_exact(&mut bytes) {
            Ok(()) => {}
            // No further HDU.
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof && block == 0 => {
                return Ok(None);
            }
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Err(corrupt("truncated header"));
            }
            Err(e) => return Err(e.into()),
        }
        for card in bytes.as_chunks::<CARD>().0 {
            let card = String::from_utf8_lossy(card);
            let key = card[..8].trim_end().to_owned();
            if key == "END" {
                return Ok(Some(Header { cards }));
            }
            if card.get(8..10) == Some("= ") {
                cards.push((key, value(&card[10..])));
            }
        }
    }
    Err(corrupt("header without END"))
}

/// A card's value: a quoted string (quotes doubled inside), or the text before a comment.
fn value(field: &str) -> String {
    let field = field.trim_start();
    if let Some(rest) = field.strip_prefix('\'') {
        let mut out = String::new();
        let mut chars = rest.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\'' {
                if chars.peek() == Some(&'\'') {
                    chars.next();
                } else {
                    break;
                }
            }
            out.push(c);
        }
        out.trim_end().to_owned()
    } else {
        field.split('/').next().unwrap_or("").trim().to_owned()
    }
}

/// Where the image is and what it holds.
struct ImageHdu {
    bitpix: i64,
    width: u32,
    height: u32,
    /// Product of the axes beyond the second.
    planes: u64,
    bzero: f64,
    bscale: f64,
    data_start: u64,
}

/// The first image: the primary HDU if it has one, else the first IMAGE extension.
fn find_image(input: &mut (impl Read + Seek)) -> Result<ImageHdu, ImportError> {
    let mut compressed = false;
    let mut start = 0u64;
    for _ in 0..MAX_HDUS {
        input.seek(SeekFrom::Start(start))?;
        let Some(header) = read_header(input)? else {
            break;
        };
        let data_start = input.stream_position()?;
        let bitpix = header.int("BITPIX").ok_or_else(|| corrupt("no BITPIX"))?;
        if ![8, 16, 32, 64, -32, -64].contains(&bitpix) {
            return Err(corrupt(&format!("BITPIX {bitpix}")));
        }
        let naxis = header.int("NAXIS").ok_or_else(|| corrupt("no NAXIS"))?;
        if !(0..=999).contains(&naxis) {
            return Err(corrupt("invalid NAXIS"));
        }
        let axes: Vec<u64> = (1..=naxis)
            .map(|i| {
                header
                    .int(&format!("NAXIS{i}"))
                    .and_then(|v| u64::try_from(v).ok())
                    .ok_or_else(|| corrupt("invalid axis"))
            })
            .collect::<Result<_, _>>()?;
        let extension = header.get("XTENSION");
        let is_image = extension.is_none_or(|x| x == "IMAGE");
        if extension == Some("BINTABLE") && header.flag("ZIMAGE") {
            compressed = true;
        }
        if is_image && axes.len() >= 2 && axes.iter().all(|&n| n > 0) {
            let size = |n: u64| u32::try_from(n).map_err(|_| corrupt("image too large"));
            return Ok(ImageHdu {
                bitpix,
                width: size(axes[0])?,
                height: size(axes[1])?,
                planes: axes[2..]
                    .iter()
                    .try_fold(1u64, |p, &n| p.checked_mul(n))
                    .ok_or_else(|| corrupt("too many planes"))?,
                bzero: header.float("BZERO").unwrap_or(0.0),
                bscale: header.float("BSCALE").unwrap_or(1.0),
                data_start,
            });
        }
        // Skip this HDU's data, padded to whole blocks.
        let elements = if axes.is_empty() {
            Some(0)
        } else {
            axes.iter().try_fold(1u64, |p, &n| p.checked_mul(n))
        }
        .and_then(|n| n.checked_add(u64::try_from(header.int("PCOUNT").unwrap_or(0)).ok()?))
        .and_then(|n| n.checked_mul(u64::try_from(header.int("GCOUNT").unwrap_or(1)).ok()?))
        .and_then(|n| n.checked_mul(bitpix.unsigned_abs() / 8))
        .ok_or_else(|| corrupt("data size overflows"))?;
        start = data_start
            .checked_add(elements.div_ceil(BLOCK) * BLOCK)
            .ok_or_else(|| corrupt("data size overflows"))?;
    }
    Err(if compressed {
        ImportError::NotYetSupported("tile-compressed FITS")
    } else {
        corrupt("no image")
    })
}

fn read(path: &Path) -> Result<(Decoded, Option<Adjustment>), ImportError> {
    let mut input = BufReader::new(File::open(path)?);
    let hdu = find_image(&mut input)?;
    let rgb = hdu.planes == 3;
    let (layout, channels) = if rgb {
        (ChannelLayout::Rgb, 3u64)
    } else {
        (ChannelLayout::Gray, 1)
    };
    let mut warnings = Vec::new();
    if !rgb && hdu.planes > 1 {
        warnings.push(ImportWarning::FirstPageOnly);
    }
    let sample = match hdu.bitpix {
        8 => SampleType::U8,
        16 => SampleType::U16,
        _ => SampleType::F32,
    };
    check_budget(hdu.width, hdu.height, layout, sample, 8 * channels as u32)?;
    let size = Size::new(hdu.width, hdu.height);
    let plane = size.pixel_count();
    let bytes = hdu.bitpix.unsigned_abs() / 8;
    let length = plane * channels * bytes;
    input.seek(SeekFrom::Start(hdu.data_start))?;
    let mut raw = Vec::new();
    (&mut input).take(length).read_to_end(&mut raw)?;
    if (raw.len() as u64) < length {
        return Err(corrupt("data too short"));
    }
    // Planes (R, G, B) to interleaved samples.
    let index = |i: usize| {
        let (pixel, channel) = (i / channels as usize, i % channels as usize);
        channel * plane as usize + pixel
    };
    let count = (plane * channels) as usize;
    let b = bytes as usize;
    let at = |i: usize| &raw[index(i) * b..index(i) * b + b];
    let pixels: Vec<u8> = match hdu.bitpix {
        8 => (0..count).map(|i| at(i)[0]).collect(),
        // Offset binary: flipping the sign bit adds 32768 (BZERO 32768 is that convention).
        16 => (0..count)
            .flat_map(|i| (u16::from_be_bytes([at(i)[0], at(i)[1]]) ^ 0x8000).to_ne_bytes())
            .collect(),
        32 | 64 => {
            warnings.push(ImportWarning::PrecisionReduced);
            let full = if b == 4 {
                f64::from(u32::MAX)
            } else {
                u64::MAX as f64
            };
            (0..count)
                .flat_map(|i| {
                    let v = if b == 4 {
                        f64::from(i32::from_be_bytes(at(i).try_into().unwrap_or([0; 4])))
                            + 2f64.powi(31)
                    } else {
                        i64::from_be_bytes(at(i).try_into().unwrap_or([0; 8])) as f64
                            + 2f64.powi(63)
                    };
                    ((v / full) as f32).to_ne_bytes()
                })
                .collect()
        }
        _ => {
            if b == 8 {
                warnings.push(ImportWarning::PrecisionReduced);
            }
            let physical: Vec<f64> = (0..count)
                .map(|i| {
                    let v = if b == 4 {
                        f64::from(f32::from_be_bytes(at(i).try_into().unwrap_or([0; 4])))
                    } else {
                        f64::from_be_bytes(at(i).try_into().unwrap_or([0; 8]))
                    };
                    hdu.bzero + hdu.bscale * v
                })
                .collect();
            let (low, high) = physical
                .iter()
                .filter(|v| v.is_finite())
                .fold((f64::MAX, f64::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
            let span = if high > low { high - low } else { 1.0 };
            warnings.push(ImportWarning::FitsValuesScaled);
            physical
                .iter()
                .flat_map(|&v| (((v - low) / span) as f32).to_ne_bytes())
                .collect()
        }
    };
    let decoded = Decoded {
        size,
        layout,
        sample,
        alpha: AlphaMode::Straight,
        icc: None,
        // Display values: the stretch maps them as they are, float ones too.
        space: Some(ColorSpace::SRGB),
        orientation: Orientation::FlipVertical,
        pixels,
        warnings,
    };
    let levels = stretch(&decoded);
    Ok((decoded, levels))
}

/// An automatic stretch, after PixInsight's STF: black a little below the background (the
/// median minus 2.8 normalized deviations), white at the lightest sample, and a gamma bringing
/// the median to a quarter of the range. Computed on at most a million samples.
fn stretch(decoded: &Decoded) -> Option<Adjustment> {
    let total = decoded.size.pixel_count() * u64::from(decoded.layout.channels());
    let step = usize::try_from(total.div_ceil(1 << 20)).unwrap_or(1).max(1);
    let mut samples: Vec<f64> = adjusted::values(decoded)
        .step_by(step)
        .filter(|v| v.is_finite())
        .collect();
    if samples.is_empty() {
        return None;
    }
    samples.sort_by(f64::total_cmp);
    let median = samples[samples.len() / 2];
    let high = samples[samples.len() - 1];
    let mut deviations: Vec<f64> = samples.iter().map(|v| (v - median).abs()).collect();
    deviations.sort_by(f64::total_cmp);
    let mad = deviations[deviations.len() / 2] * 1.4826;
    let black = (median - 2.8 * mad).max(samples[0]).clamp(0.0, 1.0);
    let white = high.clamp(0.0, 1.0);
    if white - black < 1e-6 {
        return None;
    }
    let t = (median - black) / (white - black);
    let gamma = if t > 0.0 && t < 1.0 {
        (t.ln() / 0.25f64.ln()).clamp(0.01, 9.99)
    } else {
        1.0
    };
    Some(Adjustment::Levels {
        input_black: black as f32,
        input_white: white as f32,
        gamma: gamma as f32,
        output_black: 0.0,
        output_white: 1.0,
        channels: [slopshop_core::adjust::LEVELS_IDENTITY; 3],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/fits")
            .join(name)
    }

    /// The fixtures' pattern, in the file's order (bottom row first).
    fn stored<T>(f: impl Fn(u32, u32) -> T) -> Vec<T> {
        let mut out = Vec::new();
        for r in 0..48 {
            for x in 0..64 {
                out.push(f(x, 47 - r));
            }
        }
        out
    }

    fn u16s(bytes: &[u8]) -> Vec<u16> {
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_ne_bytes(*b))
            .collect()
    }

    fn f32s(bytes: &[u8]) -> Vec<f32> {
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_ne_bytes(*b))
            .collect()
    }

    #[test]
    fn unsigned_16_bit_comes_back_exactly_flipped_upright() {
        let (decoded, levels) = read(&fixture("gray16.fits")).unwrap();
        assert_eq!(
            (decoded.layout, decoded.sample, decoded.size),
            (ChannelLayout::Gray, SampleType::U16, Size::new(64, 48))
        );
        assert_eq!(decoded.orientation, Orientation::FlipVertical);
        assert_eq!(
            u16s(&decoded.pixels),
            stored(|x, y| ((x * 1000 + y * 13) % 65536) as u16)
        );
        assert!(decoded.warnings.is_empty(), "{:?}", decoded.warnings);
        assert!(levels.is_some());
        // Upright once finished: the top-left pixel is (0, 0).
        let imported = finish(read(&fixture("gray16.fits")).unwrap().0).unwrap();
        assert_eq!(imported.image.size(), Size::new(64, 48));
    }

    #[test]
    fn float_samples_are_scaled_from_their_range_blanks_kept() {
        let (decoded, _) = read(&fixture("float.fits")).unwrap();
        assert_eq!(decoded.sample, SampleType::F32);
        assert_eq!(decoded.warnings, [ImportWarning::FitsValuesScaled]);
        let values = f32s(&decoded.pixels);
        assert!(values[0].is_nan());
        // From −10 + 2 (the blank aside, the lowest is x = 0, y = 1… stored first row is y = 47).
        let expected = stored(|x, y| -10.0 + f64::from(x) * 10.0 + f64::from(y) * 2.0);
        let finite: Vec<f64> = expected[1..].to_vec();
        let low = finite.iter().copied().fold(f64::MAX, f64::min);
        let high = finite.iter().copied().fold(f64::MIN, f64::max);
        for (i, (&v, &e)) in values.iter().zip(&expected).enumerate().skip(1) {
            let scaled = ((e - low) / (high - low)) as f32;
            assert!((v - scaled).abs() < 1e-6, "{i}: {v} vs {scaled}");
        }
    }

    #[test]
    fn three_planes_are_rgb() {
        let (decoded, _) = read(&fixture("rgb.fits")).unwrap();
        assert_eq!(decoded.layout, ChannelLayout::Rgb);
        let expected: Vec<u8> = stored(|x, y| {
            [
                (x * 4 % 256) as u8,
                (y * 5 % 256) as u8,
                ((x + y) * 3 % 256) as u8,
            ]
        })
        .concat();
        assert_eq!(decoded.pixels, expected);
    }

    #[test]
    fn images_in_extensions_and_cubes_are_found() {
        let (decoded, _) = read(&fixture("extension.fits")).unwrap();
        assert_eq!(decoded.sample, SampleType::F32);
        assert!(decoded.warnings.contains(&ImportWarning::PrecisionReduced));
        let (cube, _) = read(&fixture("cube.fits")).unwrap();
        assert_eq!(cube.warnings, [ImportWarning::FirstPageOnly]);
        assert_eq!(cube.pixels, stored(|x, _| x as u8));
    }

    #[test]
    fn opening_gives_the_image_and_its_stretch() {
        let Opened::Layers(layers) = open(&fixture("gray16.fits")).unwrap() else {
            panic!("expected layers");
        };
        let group = &layers.document.layers()[0];
        let slopshop_core::document::LayerContent::Group {
            children,
            pass_through: false,
        } = &group.content
        else {
            panic!("expected an isolated group");
        };
        assert_eq!(children[1].name, "STF auto");
        let flattened = decode(&fixture("gray16.fits")).unwrap();
        assert!(flattened.warnings.contains(&ImportWarning::LayersFlattened));
    }

    #[test]
    fn the_stretch_puts_the_median_at_a_quarter() {
        // A dark sky: most values near 0.01, a few stars at 0.9.
        let mut values = vec![0.010f32; 1000];
        for (i, v) in values.iter_mut().enumerate() {
            *v += (i % 7) as f32 * 0.0005;
        }
        values[10] = 0.9;
        let decoded = Decoded {
            size: Size::new(1000, 1),
            layout: ChannelLayout::Gray,
            sample: SampleType::F32,
            alpha: AlphaMode::Straight,
            icc: None,
            space: Some(ColorSpace::SRGB),
            orientation: Orientation::Normal,
            pixels: values.iter().flat_map(|v| v.to_ne_bytes()).collect(),
            warnings: Vec::new(),
        };
        let Some(Adjustment::Levels {
            input_black,
            input_white,
            gamma,
            ..
        }) = stretch(&decoded)
        else {
            panic!("no stretch");
        };
        assert!((input_white - 0.9).abs() < 1e-6);
        let median = 0.0115f32;
        let t = (median - input_black) / (input_white - input_black);
        assert!((t.powf(1.0 / gamma) - 0.25).abs() < 0.02, "{gamma}");
    }

    #[test]
    fn headers_values_and_damaged_files() {
        assert_eq!(value("                   16 / comment"), "16");
        assert_eq!(value("'IMAGE   '           / type"), "IMAGE");
        assert_eq!(value("'it''s'"), "it's");
        let bytes = std::fs::read(fixture("gray16.fits")).unwrap();
        assert!(is_fits(&bytes));
        let path =
            std::env::temp_dir().join(format!("slopshop-fits-{}-cut.fits", std::process::id()));
        std::fs::write(&path, &bytes[..4000]).unwrap();
        let result = read(&path);
        std::fs::remove_file(&path).ok();
        assert!(result.is_err());
    }
}
