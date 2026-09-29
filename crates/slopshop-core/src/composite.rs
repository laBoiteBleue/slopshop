//! CPU reference compositor (ADR 0008): the test oracle for the GPU renderer, and the export
//! path when no GPU is available.
//!
//! It produces a region of the document at full resolution (pyramid level 0, never coarser), as
//! premultiplied RGBA `f32` in the working space: visible layers from bottom to top, combined
//! with premultiplied "over". Raster texels are decoded exactly like the raster codec does on
//! import, except for the display clamp: finite values are kept as they are, however large.
//! Only non-finite values are replaced, and counted: NaN reads as 0, and ±inf as
//! ±[`MAX_FINITE_SAMPLE`] (the display bound, also the largest half float). Not ±`f32::MAX`:
//! through a color matrix, such a value would swamp the other channels of the pixel (f32
//! rounding at 1e38 is about 1e31), while at ±65504 they keep about 1e-2 of absolute accuracy
//! through a round trip between gamuts. The GPU export path (`export_main` in the renderer)
//! applies the same rule.
//!
//! Accumulation is done in `f64` so that huge values cannot overflow into infinities (and then
//! NaN, as `inf × 0`) between layers; results beyond the `f32` range saturate to ±`f32::MAX`,
//! and are counted too.

use std::fmt;

use crate::color::{IDENTITY, Mat3, mat_vec};
use crate::document::{Document, LayerContent};
use crate::geom::{Rect, Size};
use crate::raster::{Codec, MAX_FINITE_SAMPLE, RasterLevel, TILE_SIZE};
use crate::tile::TileCoord;

/// Lossy events of a composite.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompositeReport {
    /// Non-finite values replaced: NaN and ±inf raster samples (or decoded values), and
    /// composited values beyond the `f32` range, saturated. One infinity may count more than
    /// once (e.g. a sample, then the composite it overflows).
    pub non_finite: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CompositeError {
    /// The region is not inside the document.
    RegionOutOfBounds { region: Rect, size: Size },
    /// The output buffer does not hold exactly `width × height × 4` values.
    BufferSizeMismatch { expected: u64, actual: u64 },
}

impl fmt::Display for CompositeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompositeError::RegionOutOfBounds { region, size } => write!(
                f,
                "region {region:?} is outside the {}×{} document",
                size.width, size.height
            ),
            CompositeError::BufferSizeMismatch { expected, actual } => {
                write!(f, "output buffer has {actual} values, expected {expected}")
            }
        }
    }
}

impl std::error::Error for CompositeError {}

/// A visible layer, ready to be sampled.
enum Source<'a> {
    /// Premultiplied working-space color, opacity applied.
    Fill([f64; 4]),
    Raster {
        level: &'a RasterLevel,
        codec: Codec,
        /// Image space → working space; `None` for the identity (keeps huge values exact).
        matrix: Option<Mat3>,
        opacity: f64,
    },
}

/// Composite `region` of `document` into `out`: `region.width × region.height` pixels, row-major,
/// premultiplied RGBA `f32` in the document's working space. Rows are computed in parallel.
pub fn composite_region(
    document: &Document,
    region: Rect,
    out: &mut [f32],
) -> Result<CompositeReport, CompositeError> {
    let size = document.size();
    if region.right() > u64::from(size.width) || region.bottom() > u64::from(size.height) {
        return Err(CompositeError::RegionOutOfBounds { region, size });
    }
    let expected = region.size().pixel_count() * 4;
    if out.len() as u64 != expected {
        return Err(CompositeError::BufferSizeMismatch {
            expected,
            actual: out.len() as u64,
        });
    }
    if region.is_empty() {
        return Ok(CompositeReport::default());
    }

    let working = document.working_space();
    let sources: Vec<Source> = document
        .layers()
        .iter()
        .filter(|layer| layer.visible && layer.opacity > 0.0)
        .filter_map(|layer| match &layer.content {
            LayerContent::Fill { color } => {
                let a = f64::from(color.a) * f64::from(layer.opacity);
                Some(Source::Fill([
                    f64::from(color.r) * a,
                    f64::from(color.g) * a,
                    f64::from(color.b) * a,
                    a,
                ]))
            }
            LayerContent::Raster { image } => {
                let matrix = image.matrix_to(&working);
                Some(Source::Raster {
                    level: image.levels().first()?,
                    codec: Codec::new(image.stored_format()),
                    matrix: (matrix != IDENTITY).then_some(matrix),
                    opacity: f64::from(layer.opacity),
                })
            }
        })
        .collect();

    let width = region.width as usize;
    let row_len = width * 4;
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let rows_per_chunk = (region.height as usize).div_ceil(threads).max(1);
    let chunks: Vec<&mut [f32]> = out.chunks_mut(rows_per_chunk * row_len).collect();
    let mut reports = vec![CompositeReport::default(); chunks.len()];
    std::thread::scope(|scope| {
        for (chunk_index, (chunk, report)) in chunks.into_iter().zip(&mut reports).enumerate() {
            let sources = &sources;
            scope.spawn(move || {
                let mut acc = vec![[0.0f64; 4]; width];
                for (i, row) in chunk.chunks_exact_mut(row_len).enumerate() {
                    // Fits: the row is inside the region, whose bottom fits the document.
                    let y = region.y + (chunk_index * rows_per_chunk + i) as u32;
                    composite_row(sources, region.x, y, &mut acc, report);
                    for (value, &v) in row.iter_mut().zip(acc.iter().flatten()) {
                        *value = saturate(v, report);
                    }
                }
            });
        }
    });
    Ok(CompositeReport {
        non_finite: reports.iter().map(|r| r.non_finite).sum(),
    })
}

/// Composite the pixels `x0..x0 + acc.len()` of row `y` into `acc`.
fn composite_row(
    sources: &[Source],
    x0: u32,
    y: u32,
    acc: &mut [[f64; 4]],
    report: &mut CompositeReport,
) {
    acc.fill([0.0; 4]);
    for source in sources {
        match source {
            Source::Fill(color) => {
                for dst in acc.iter_mut() {
                    over(color, dst);
                }
            }
            Source::Raster {
                level,
                codec,
                matrix,
                opacity,
            } => {
                let size = level.size();
                // The raster sits at the origin and may be smaller: transparent outside.
                if y >= size.height || x0 >= size.width {
                    continue;
                }
                let end = (u64::from(x0) + acc.len() as u64).min(u64::from(size.width)) as u32;
                let (row, local_y) = (y / TILE_SIZE, (y % TILE_SIZE) as usize);
                let mut x = x0;
                // One tile-wide run at a time.
                while x < end {
                    let col = x / TILE_SIZE;
                    let run_end = end.min((col + 1).saturating_mul(TILE_SIZE));
                    if let Some(tile) = level.tile(TileCoord { col, row }) {
                        for px_x in x..run_end {
                            let local_x = (px_x % TILE_SIZE) as usize;
                            let start =
                                (local_y * TILE_SIZE as usize + local_x) * codec.bytes_per_pixel;
                            let Some(px) = tile.get(start..start + codec.bytes_per_pixel) else {
                                continue;
                            };
                            let src = texel(codec, px, matrix.as_ref(), *opacity, report);
                            over(&src, &mut acc[(px_x - x0) as usize]);
                        }
                    }
                    x = run_end;
                }
            }
        }
    }
}

/// Premultiplied working-space color of one stored pixel, opacity applied.
fn texel(
    codec: &Codec,
    px: &[u8],
    matrix: Option<&Mat3>,
    opacity: f64,
    report: &mut CompositeReport,
) -> [f64; 4] {
    let mut map = |v: f32| {
        if v.is_finite() {
            v
        } else {
            report.non_finite += 1;
            if v.is_nan() {
                0.0
            } else {
                MAX_FINITE_SAMPLE.copysign(v)
            }
        }
    };
    let (color, alpha) = codec.read_mapped(px, &mut map);
    let color = color.map(f64::from);
    let [r, g, b] = match matrix {
        Some(m) => mat_vec(m, color),
        None => color,
    };
    [r, g, b, f64::from(alpha)].map(|v| v * opacity)
}

/// Premultiplied "over": `dst = src + dst × (1 − src.a)`.
fn over(src: &[f64; 4], dst: &mut [f64; 4]) {
    let keep = 1.0 - src[3];
    for (d, s) in dst.iter_mut().zip(src) {
        *d = s + *d * keep;
    }
}

/// A composited value as `f32`, saturating (and counting) beyond the `f32` range.
fn saturate(v: f64, report: &mut CompositeReport) -> f32 {
    let x = v as f32;
    if x.is_finite() {
        x
    } else {
        report.non_finite += 1;
        if v.is_nan() {
            0.0
        } else {
            f32::MAX.copysign(x)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::color::{
        AlphaMode, ChannelLayout, ColorSpace, LinearRgba, PixelFormat, SampleType,
        TransferFunction, WORKING_SPACE, srgb_decode,
    };
    use crate::convert::{ConversionReport, ConvertOptions, Converter};
    use crate::document::{Layer, LayerId};
    use crate::edit::Edit;
    use crate::raster::RasterImage;

    fn add(doc: &mut Document, content: LayerContent, opacity: f32, visible: bool) -> LayerId {
        let id = doc.allocate_layer_id();
        let index = doc.layers().len();
        Edit::InsertLayer {
            index,
            layer: Layer {
                id,
                name: format!("layer {}", id.get()),
                visible,
                opacity,
                content,
            },
        }
        .apply(doc)
        .unwrap();
        id
    }

    fn fill(r: f32, g: f32, b: f32, a: f32) -> LayerContent {
        LayerContent::Fill {
            color: LinearRgba::new(r, g, b, a),
        }
    }

    fn raster(size: Size, format: PixelFormat, pixels: &[u8]) -> LayerContent {
        LayerContent::Raster {
            image: Arc::new(RasterImage::from_pixels(size, format, pixels).unwrap()),
        }
    }

    fn float_raster(size: Size, pixels: &[[f32; 4]]) -> LayerContent {
        let format = PixelFormat {
            layout: ChannelLayout::Rgba,
            sample: SampleType::F32,
            color_space: WORKING_SPACE,
            alpha: AlphaMode::Straight,
        };
        let bytes: Vec<u8> = pixels
            .iter()
            .flatten()
            .flat_map(|v| v.to_ne_bytes())
            .collect();
        raster(size, format, &bytes)
    }

    fn composite(doc: &Document, region: Rect) -> (Vec<f32>, CompositeReport) {
        let mut out = vec![0.0; region.size().pixel_count() as usize * 4];
        let report = composite_region(doc, region, &mut out).unwrap();
        (out, report)
    }

    fn assert_close(actual: &[f32], expected: &[f32]) {
        assert_eq!(actual.len(), expected.len());
        for (a, e) in actual.iter().zip(expected) {
            assert!((a - e).abs() < 1e-6, "{actual:?} != {expected:?}");
        }
    }

    #[test]
    fn layers_combine_with_opacity_and_visibility() {
        let mut doc = Document::new(Size::new(3, 1));
        add(&mut doc, fill(0.2, 0.4, 0.6, 1.0), 1.0, true);
        // A 2 × 1 raster: straight (1, 0.5, 0, 0.5) then (0, 0, 0, 0), at 50 % opacity.
        let pixels = [[1.0, 0.5, 0.0, 0.5], [0.0; 4]];
        add(&mut doc, float_raster(Size::new(2, 1), &pixels), 0.5, true);
        add(&mut doc, fill(1.0, 0.0, 0.0, 1.0), 1.0, false);
        add(&mut doc, fill(0.0, 0.0, 1.0, 0.5), 0.5, true);
        let (out, report) = composite(&doc, doc.size().bounds());
        // Raster: (0.25, 0.125, 0, 0.25) over the fill: (0.4, 0.425, 0.45, 1). Then the top
        // fill, (0, 0, 0.25, 0.25) premultiplied: × 0.75 below, + 0.25 blue.
        let expected = [
            [0.3, 0.318_75, 0.5875, 1.0],
            // Transparent raster pixel, then outside the raster: the bottom fill shows.
            [0.15, 0.3, 0.7, 1.0],
            [0.15, 0.3, 0.7, 1.0],
        ];
        assert_close(&out, expected.as_flattened());
        assert_eq!(report, CompositeReport::default());

        // An empty document is transparent.
        let empty = Document::new(Size::new(2, 2));
        assert_eq!(composite(&empty, empty.size().bounds()).0, [0.0; 16]);
    }

    #[test]
    fn regions_match_the_full_composite() {
        // Height not a multiple of 256, a raster smaller than the document, crossing tiles.
        let size = Size::new(600, 300);
        let raster_size = Size::new(520, 270);
        let pixels: Vec<u8> = (0..raster_size.pixel_count() as usize)
            .flat_map(|i| {
                let (x, y) = (i % 520, i / 520);
                [
                    (x % 256) as u8,
                    (y % 256) as u8,
                    ((x * 7 + y * 3) % 256) as u8,
                    (64 + (x + y) % 192) as u8,
                ]
            })
            .collect();
        let mut doc = Document::new(size);
        add(&mut doc, fill(0.1, 0.2, 0.3, 0.5), 1.0, true);
        add(
            &mut doc,
            raster(raster_size, PixelFormat::RGBA8_SRGB, &pixels),
            0.8,
            true,
        );
        let (full, _) = composite(&doc, size.bounds());

        // Every pixel against an independent computation.
        let to_working = ColorSpace::LINEAR_SRGB.matrix_to(&WORKING_SPACE);
        for y in 0..300usize {
            for x in 0..600usize {
                let mut expected = [0.05f64, 0.1, 0.15, 0.5];
                if x < 520 && y < 270 {
                    let px = &pixels[(y * 520 + x) * 4..][..4];
                    let a = f64::from(px[3]) / 255.0;
                    let linear =
                        [0, 1, 2].map(|c| f64::from(srgb_decode(f32::from(px[c]) / 255.0)));
                    let [r, g, b] = mat_vec(&to_working, linear);
                    let src = [r * a, g * a, b * a, a].map(|v| v * 0.8);
                    over(&src, &mut expected);
                }
                let actual = &full[(y * 600 + x) * 4..][..4];
                for c in 0..4 {
                    assert!(
                        (f64::from(actual[c]) - expected[c]).abs() < 1e-5,
                        "({x}, {y}): {actual:?} vs {expected:?}"
                    );
                }
            }
        }

        for region in [
            Rect::new(250, 200, 20, 70),
            Rect::new(500, 0, 100, 300),
            Rect::new(0, 256, 600, 44),
            Rect::new(255, 255, 2, 2),
            Rect::new(599, 299, 1, 1),
        ] {
            let (part, _) = composite(&doc, region);
            for row in 0..region.height as usize {
                let y = region.y as usize + row;
                let start = (y * 600 + region.x as usize) * 4;
                let expected = &full[start..start + region.width as usize * 4];
                assert_eq!(
                    &part[row * region.width as usize * 4..][..region.width as usize * 4],
                    expected,
                    "{region:?} row {row}"
                );
            }
        }
    }

    #[test]
    fn large_float_values_are_not_clamped() {
        let mut doc = Document::new(Size::new(2, 1));
        let pixels = [[1e5, -3.0, 0.5, 1.0], [3e38, 1.0, 1.0, 1.0]];
        add(&mut doc, float_raster(Size::new(2, 1), &pixels), 1.0, true);
        let (out, report) = composite(&doc, doc.size().bounds());
        assert_eq!(out, pixels.as_flattened());
        assert_eq!(report.non_finite, 0);
        // The display path still clamps (see MAX_FINITE_SAMPLE); export does not.
    }

    #[test]
    fn non_finite_samples_are_mapped_and_counted() {
        let mut doc = Document::new(Size::new(3, 1));
        add(&mut doc, fill(0.5, 0.5, 0.5, 1.0), 1.0, true);
        let pixels = [
            [f32::INFINITY, f32::NAN, 0.25, 1.0],
            [f32::NEG_INFINITY, 0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0, f32::NAN],
        ];
        add(&mut doc, float_raster(Size::new(3, 1), &pixels), 1.0, true);
        let (out, report) = composite(&doc, doc.size().bounds());
        assert_eq!(
            out,
            [
                [MAX_FINITE_SAMPLE, 0.0, 0.25, 1.0],
                [-MAX_FINITE_SAMPLE, 0.0, 0.0, 1.0],
                // NaN alpha is 0: the fill below shows through.
                [0.5, 0.5, 0.5, 1.0],
            ]
            .as_flattened()
        );
        assert_eq!(report.non_finite, 4);
        // A region only counts what it covers.
        let (_, report) = composite(&doc, Rect::new(1, 0, 1, 1));
        assert_eq!(report.non_finite, 1);
    }

    /// Regression: ±inf used to be read as ±`f32::MAX`, and the matrices to the working space
    /// and back turned the other channels of the pixel (0.5) into ±1e30.
    #[test]
    fn an_infinite_sample_keeps_the_other_channels_of_its_pixel() {
        let size = Size::new(3, 1);
        let format = PixelFormat {
            layout: ChannelLayout::Rgba,
            sample: SampleType::F32,
            color_space: ColorSpace::LINEAR_SRGB,
            alpha: AlphaMode::Premultiplied,
        };
        let pixels = [
            [f32::NAN, 0.5, 0.5, 1.0],
            [f32::INFINITY, 0.5, 0.5, 1.0],
            [0.5, f32::NEG_INFINITY, 0.5, 1.0],
        ];
        let bytes: Vec<u8> = pixels
            .iter()
            .flatten()
            .flat_map(|v| v.to_ne_bytes())
            .collect();
        let mut doc = Document::new(size);
        add(&mut doc, raster(size, format, &bytes), 1.0, true);
        let (out, report) = composite(&doc, size.bounds());
        assert_eq!(report.non_finite, 3);
        // Back to linear sRGB, as an export to that space does.
        let back = WORKING_SPACE.matrix_to(&ColorSpace::LINEAR_SRGB);
        let expected = [
            [0.0, 0.5, 0.5],
            [f64::from(MAX_FINITE_SAMPLE), 0.5, 0.5],
            [0.5, -f64::from(MAX_FINITE_SAMPLE), 0.5],
        ];
        for (px, expected) in out.as_chunks::<4>().0.iter().zip(expected) {
            let rgb = mat_vec(&back, [px[0], px[1], px[2]].map(f64::from));
            for (v, e) in rgb.iter().zip(expected) {
                assert!(
                    (v - e).abs() < 0.02,
                    "{px:?} → {rgb:?}, expected {expected:?}"
                );
            }
            assert_eq!(px[3], 1.0);
        }
    }

    #[test]
    fn rejects_bad_regions_and_buffers() {
        let doc = Document::new(Size::new(10, 10));
        let mut out = vec![0.0; 4 * 4 * 4];
        assert!(matches!(
            composite_region(&doc, Rect::new(8, 0, 4, 4), &mut out),
            Err(CompositeError::RegionOutOfBounds { .. })
        ));
        assert!(matches!(
            composite_region(&doc, Rect::new(0, 0, 4, 3), &mut out),
            Err(CompositeError::BufferSizeMismatch { .. })
        ));
        assert_eq!(
            composite_region(&doc, Rect::new(10, 10, 0, 0), &mut []),
            Ok(CompositeReport::default())
        );
    }

    /// Composite a single-raster document and convert it back to the source format.
    fn export_single_raster(
        size: Size,
        format: PixelFormat,
        pixels: &[u8],
        dither: bool,
    ) -> (Vec<u8>, ConversionReport) {
        let mut doc = Document::new(size);
        add(&mut doc, raster(size, format, pixels), 1.0, true);
        let (linear, report) = composite(&doc, size.bounds());
        assert_eq!(report.non_finite, 0);
        let options = ConvertOptions {
            dither,
            big_endian: false,
        };
        let converter = Converter::new(format, options).unwrap();
        let bpp = converter.bytes_per_pixel();
        let row_len = size.width as usize;
        let mut out = vec![0u8; size.pixel_count() as usize * bpp];
        let mut conversion = ConversionReport::default();
        for (y, (src, dst)) in linear
            .chunks_exact(row_len * 4)
            .zip(out.chunks_exact_mut(row_len * bpp))
            .enumerate()
        {
            converter
                .convert_row(src, 0, y as u32, dst, &mut conversion)
                .unwrap();
        }
        (out, conversion)
    }

    /// Every named color space.
    const NAMED_SPACES: [ColorSpace; 9] = [
        ColorSpace::SRGB,
        ColorSpace::LINEAR_SRGB,
        ColorSpace::DISPLAY_P3,
        ColorSpace::ADOBE_RGB,
        ColorSpace::PROPHOTO,
        ColorSpace::REC2020,
        ColorSpace::LINEAR_REC2020,
        ColorSpace::REC2100_PQ,
        ColorSpace::REC2100_HLG,
    ];

    #[test]
    fn unedited_integer_sources_export_bit_exact() {
        // What `convert`'s module documentation promises, on the CPU path: an unedited source
        // exported in its own format comes back bit-exact, with an empty report, except for the
        // lowest 16-bit codes of pure power curves.

        // 8-bit: every value in every channel, at every non-zero alpha, in every named space.
        let size = Size::new(256, 255);
        let pixels: Vec<u8> = (0..size.pixel_count() as usize)
            .flat_map(|i| {
                let (x, y) = ((i % 256) as u8, (i / 256) as u8);
                [x, 255 - x, x.wrapping_mul(7).wrapping_add(y), y + 1]
            })
            .collect();
        for space in NAMED_SPACES {
            let format = PixelFormat {
                color_space: space,
                ..PixelFormat::RGBA8_SRGB
            };
            for dither in [false, true] {
                let (out, report) = export_single_raster(size, format, &pixels, dither);
                assert!(out == pixels, "8-bit {space:?}, dither {dither}");
                assert_eq!(report, ConversionReport::default(), "8-bit {space:?}");
            }
        }

        // 16-bit: every code in every channel, in every named space.
        let size = Size::new(256, 256);
        let codes: Vec<u16> = (0..=u16::MAX)
            .flat_map(|v| [v, v.wrapping_mul(31), u16::MAX - v, 65535 - (v % 7) * 1000])
            .collect();
        let pixels: Vec<u8> = codes.iter().flat_map(|c| c.to_ne_bytes()).collect();
        for space in NAMED_SPACES {
            let format = PixelFormat {
                layout: ChannelLayout::Rgba,
                sample: SampleType::U16,
                color_space: space,
                alpha: AlphaMode::Straight,
            };
            let (out, report) = export_single_raster(size, format, &pixels, false);
            assert_eq!(report, ConversionReport::default(), "16-bit {space:?}");
            // The converter writes little-endian.
            let wrong: Vec<(u16, u16)> = codes
                .iter()
                .zip(out.as_chunks::<2>().0)
                .map(|(&expected, bytes)| (expected, u16::from_le_bytes(*bytes)))
                .filter(|(expected, actual)| expected != actual)
                .collect();
            match space.transfer {
                // Infinite slope at 0: the lowest codes are closer together than the rounding
                // of the working space. Measured: Adobe RGB 74 samples wrong (codes up to 70,
                // off by up to 6), ProPhoto 2 (codes up to 2, off by up to 2).
                TransferFunction::Gamma(_) => {
                    assert!(wrong.len() < 256, "16-bit {space:?}: {} wrong", wrong.len());
                    for (expected, actual) in wrong {
                        assert!(
                            expected < 256 && actual.abs_diff(expected) <= 8,
                            "16-bit {space:?}: {expected} became {actual}"
                        );
                    }
                }
                _ => assert!(wrong.is_empty(), "16-bit {space:?}: {:?}…", wrong.first()),
            }
        }
    }
}
