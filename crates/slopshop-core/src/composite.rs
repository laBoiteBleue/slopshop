//! CPU reference compositor (ADR 0008): the test oracle for the GPU renderer, and the export
//! path when no GPU is available.
//!
//! It produces a region of the document at full resolution (pyramid level 0, never coarser), as
//! premultiplied RGBA `f32` in the working space: visible layers from bottom to top, each
//! combined with what is below by its blend mode, in the document's blend space (ADR 0012),
//! after its mask (ADR 0014), groups through a stack of accumulators ([`steps`], ADR 0015).
//! Transformed layers are resampled (ADR 0018, [`crate::resample`]); whole-pixel moves are not.
//! Raster texels are decoded exactly like the raster codec does on
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

use crate::adjust::Adjustment;
use crate::blend::{BlendMode, Blender, dissolve};
use crate::color::WORKING_SPACE;
use crate::color::{IDENTITY, Mat3, mat_vec};
use crate::document::{Document, Layer, LayerContent, LayerMask};
use crate::geom::{Rect, Size};
use crate::raster::{Codec, MAX_FINITE_SAMPLE, RasterImage, RasterLevel, TILE_SIZE};
use crate::resample::{Resampling, TABLE_SIZE, weight_table};
use crate::tile::TileCoord;
use crate::transform::Affine;

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

/// One step of compositing (ADR 0015, 0016): the visible layer tree flattened into a single pass
/// over a stack of accumulators, shared by this compositor and the GPU renderer. Steps carry the
/// mode and opacity to apply, resolved from the layers (a clipping base is drawn in normal mode
/// at full opacity, its own applying to its clipping group).
#[derive(Debug, Clone, Copy)]
pub enum Step<'a> {
    /// Blend a fill or raster layer (its content and mask) onto the accumulator with `mode` and
    /// `opacity`; `atop` (a clipped layer) keeps the accumulator's coverage.
    Layer {
        layer: &'a Layer,
        mode: BlendMode,
        opacity: f32,
        atop: bool,
        /// From the layer's content (and mask) to the document: its transform composed with its
        /// groups' (ADR 0017).
        transform: Affine,
    },
    /// Apply an adjustment layer to the accumulator (ADR 0020), mixed with it by `opacity` × its
    /// mask (placed by `transform`).
    Adjust {
        layer: &'a Layer,
        adjustment: Adjustment,
        opacity: f32,
        transform: Affine,
    },
    /// Push the accumulator. An isolated group starts over from transparency; a pass-through
    /// one keeps compositing onto what is below it.
    Begin { isolated: bool },
    /// Pop what was pushed and combine the result with it: faded by `opacity` × `mask`
    /// (pass-through), or blended as one layer with `mode`, `opacity` and `mask` (isolated),
    /// atop what was pushed when `atop`.
    End {
        mask: Option<&'a LayerMask>,
        /// From the mask to the document: the group's composed transform.
        mask_transform: Affine,
        mode: BlendMode,
        opacity: f32,
        isolated: bool,
        atop: bool,
    },
}

/// The steps that composite `document`, bottom to top. Hidden layers, layers at opacity 0 and
/// groups without visible content are left out; a pass-through group at full opacity without an
/// enabled mask changes nothing to its children's result, so they are inlined.
pub fn steps(document: &Document) -> Vec<Step<'_>> {
    let mut steps = Vec::new();
    push_steps(document.layers(), Affine::IDENTITY, &mut steps);
    steps
}

/// How a layer takes part in compositing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Plain,
    /// The base of a clipping group: normal mode, full opacity (they apply to the group).
    Base,
    /// Clipped: atop its clipping group.
    Clipped,
}

fn shown(layer: &Layer) -> bool {
    layer.visible && layer.opacity > 0.0
}

fn enabled(layer: &Layer) -> Option<&LayerMask> {
    layer.mask.as_ref().filter(|m| m.enabled)
}

/// Steps of sibling `layers` (in a space mapped to the document by `parent`): each base with the
/// clipped layers above it (ADR 0016).
fn push_steps<'a>(layers: &'a [Layer], parent: Affine, steps: &mut Vec<Step<'a>>) {
    let mut i = 0;
    while i < layers.len() {
        // A layer and the clipped layers above it (a clipped layer without a base is drawn as
        // usual: it is the first of its level).
        let base = &layers[i];
        let mut end = i + 1;
        while end < layers.len() && layers[end].clipped {
            end += 1;
        }
        let clipped: Vec<&Layer> = layers[i + 1..end].iter().filter(|l| shown(l)).collect();
        i = end;
        // A hidden base hides its clipping group.
        if !shown(base) {
            continue;
        }
        if clipped.is_empty() {
            push_layer(base, Role::Plain, parent, steps);
            continue;
        }
        let start = steps.len();
        steps.push(Step::Begin { isolated: true });
        push_layer(base, Role::Base, parent, steps);
        if steps.len() == start + 1 {
            // The base draws nothing: nothing shows through it.
            steps.truncate(start);
            continue;
        }
        for layer in clipped {
            push_layer(layer, Role::Clipped, parent, steps);
        }
        steps.push(Step::End {
            mask: None,
            mask_transform: Affine::IDENTITY,
            mode: group_mode(base),
            opacity: base.opacity,
            isolated: true,
            atop: false,
        });
    }
}

/// The mode a layer blends its result with: normal for a pass-through group.
fn group_mode(layer: &Layer) -> BlendMode {
    match layer.content {
        LayerContent::Group {
            pass_through: true, ..
        } => BlendMode::Normal,
        _ => layer.blend_mode,
    }
}

fn push_layer<'a>(layer: &'a Layer, role: Role, parent: Affine, steps: &mut Vec<Step<'a>>) {
    let transform = layer.transform.then(parent);
    let (mode, opacity) = match role {
        Role::Base => (BlendMode::Normal, 1.0),
        Role::Plain | Role::Clipped => (layer.blend_mode, layer.opacity),
    };
    let atop = role == Role::Clipped;
    if let LayerContent::Adjustment { adjustment } = &layer.content {
        steps.push(Step::Adjust {
            layer,
            adjustment: *adjustment,
            opacity,
            transform,
        });
        return;
    }
    let LayerContent::Group {
        children,
        pass_through,
    } = &layer.content
    else {
        steps.push(Step::Layer {
            layer,
            mode,
            opacity,
            atop,
            transform,
        });
        return;
    };
    // A base or a clipped group is composited as a unit: isolated.
    let passes = *pass_through && role == Role::Plain;
    let mask = enabled(layer);
    if passes && opacity >= 1.0 && mask.is_none() {
        push_steps(children, transform, steps);
        return;
    }
    let start = steps.len();
    steps.push(Step::Begin { isolated: !passes });
    push_steps(children, transform, steps);
    if steps.len() == start + 1 {
        // Nothing visible inside: the group changes nothing.
        steps.truncate(start);
        return;
    }
    steps.push(Step::End {
        mask,
        mask_transform: transform,
        mode: if *pass_through {
            BlendMode::Normal
        } else {
            mode
        },
        opacity,
        isolated: !passes,
        atop,
    });
}

/// What a row pass does at each step.
enum Op<'a> {
    /// Boxed: much larger than the other steps.
    Layer(Box<Source<'a>>),
    Adjust {
        adjustment: crate::adjust::Prepared,
        opacity: f64,
        mask: Option<MaskSource<'a>>,
    },
    Begin {
        isolated: bool,
    },
    End {
        mode: BlendMode,
        opacity: f64,
        mask: Option<MaskSource<'a>>,
        isolated: bool,
        atop: bool,
    },
}

/// A mask image, read as coverage, placed by `transform`.
fn mask_source(mask: &LayerMask, transform: Affine) -> Option<MaskSource<'_>> {
    let (level, placement) = placement(&mask.image, transform)?;
    Some(MaskSource {
        level,
        codec: Codec::new(mask.image.stored_format()),
        placement,
    })
}

/// Where a raster is in the document (ADR 0017, 0018).
#[derive(Debug, Clone)]
enum Placement {
    /// A whole-pixel offset: level-0 texels are document pixels.
    Offset(i64, i64),
    /// Any other transform: resampled (boxed: much larger than an offset).
    Resampled(Box<Resampling>),
}

/// How `image`, placed by `transform`, is sampled: the level read and the placement. `None`
/// for a transform that is not invertible (edits refuse them).
fn placement(image: &RasterImage, transform: Affine) -> Option<(&RasterLevel, Placement)> {
    let levels = image.levels();
    match transform.integer_translation() {
        Some((x, y)) => Some((levels.first()?, Placement::Offset(x, y))),
        None => {
            let r = Resampling::new(transform, 1.0, levels.len())?;
            Some((levels.get(r.level)?, Placement::Resampled(Box::new(r))))
        }
    }
}

/// The stored bytes of texel (`i`, `j`) of `level`: `None` outside the level, `Some(None)` in a
/// tile that is not stored (transparent).
fn stored_texel(
    level: &RasterLevel,
    bytes_per_pixel: usize,
    i: i64,
    j: i64,
) -> Option<Option<&[u8]>> {
    let size = level.size();
    if i < 0 || j < 0 || i >= i64::from(size.width) || j >= i64::from(size.height) {
        return None;
    }
    // In range just above.
    let (x, y) = (i as u32, j as u32);
    let coord = TileCoord {
        col: x / TILE_SIZE,
        row: y / TILE_SIZE,
    };
    let start = ((y % TILE_SIZE) * TILE_SIZE + x % TILE_SIZE) as usize * bytes_per_pixel;
    Some(
        level
            .tile(coord)
            .and_then(|tile| tile.get(start..start + bytes_per_pixel)),
    )
}

/// A fill or raster layer, placed by `transform`, ready to be sampled with `mode` and
/// `opacity` (atop when `atop`).
fn source(
    layer: &Layer,
    mode: BlendMode,
    opacity: f32,
    atop: bool,
    transform: Affine,
) -> Option<Source<'_>> {
    let replaces_alpha = layer.mask.as_ref().is_some_and(|m| m.replaces_alpha);
    let content = match &layer.content {
        LayerContent::Fill { color } => {
            let alpha = if replaces_alpha {
                1.0
            } else {
                f64::from(color.a)
            };
            let a = alpha * f64::from(opacity);
            SourceContent::Fill([
                f64::from(color.r) * a,
                f64::from(color.g) * a,
                f64::from(color.b) * a,
                a,
            ])
        }
        LayerContent::Raster { image } => {
            let matrix = image.matrix_to(&WORKING_SPACE);
            let (level, placement) = placement(image, transform)?;
            SourceContent::Raster {
                level,
                codec: Codec::new(image.stored_format()),
                matrix: (matrix != IDENTITY).then_some(matrix),
                opacity: f64::from(opacity),
                placement,
            }
        }
        // Groups and adjustments are steps of their own.
        LayerContent::Group { .. } | LayerContent::Adjustment { .. } => return None,
    };
    Some(Source {
        mode,
        atop,
        content,
        replaces_alpha,
        mask: enabled(layer).and_then(|m| mask_source(m, transform)),
    })
}

/// A visible layer, ready to be sampled, with its blend mode and mask.
struct Source<'a> {
    mode: BlendMode,
    /// Clipped: blended atop its clipping group (ADR 0016).
    atop: bool,
    content: SourceContent<'a>,
    /// The layer's own alpha is ignored (a mask made from its transparency, ADR 0014).
    replaces_alpha: bool,
    /// An enabled mask.
    mask: Option<MaskSource<'a>>,
}

/// A mask image, read as coverage, placed in the document.
struct MaskSource<'a> {
    /// The level sampled: level 0 unless resampled.
    level: &'a RasterLevel,
    codec: Codec,
    placement: Placement,
}

impl MaskSource<'_> {
    /// Coverage at document pixel (`x`, `y`): the mask sample (resampled if the mask is
    /// transformed), linear, clamped to `[0, 1]`; 0 outside the mask image. Non-finite samples
    /// are not counted (the GPU does the same): NaN reads as 0, ±inf as ±[`MAX_FINITE_SAMPLE`].
    fn coverage(&self, x: u32, y: u32, table: &[f32; TABLE_SIZE]) -> f64 {
        let bytes = self.codec.bytes_per_pixel;
        let value = match &self.placement {
            &Placement::Offset(ox, oy) => {
                match stored_texel(self.level, bytes, i64::from(x) - ox, i64::from(y) - oy) {
                    Some(Some(px)) => self.value(px),
                    _ => 0.0,
                }
            }
            Placement::Resampled(r) => {
                let size = self.level.size();
                let p = (f64::from(x) + 0.5, f64::from(y) + 0.5);
                if !r.reaches(p, size.width, size.height) {
                    return 0.0;
                }
                let (color, _) = r.sample(table, p, |i, j| {
                    stored_texel(self.level, bytes, i, j)
                        .map(|px| [px.map_or(0.0, |px| self.value(px)); 4])
                });
                color[0]
            }
        };
        value.clamp(0.0, 1.0)
    }

    /// A stored mask sample, non-finite values mapped.
    fn value(&self, px: &[u8]) -> f64 {
        let (color, _) = self.codec.read_mapped(px, &mut |v| {
            if v.is_nan() {
                0.0
            } else if v.is_infinite() {
                MAX_FINITE_SAMPLE.copysign(v)
            } else {
                v
            }
        });
        f64::from(color[0])
    }
}

/// A layer's color with its alpha ignored: straight color, alpha 1 (then opacity).
fn opaque(premultiplied: [f64; 4], opacity: f64) -> [f64; 4] {
    let a = premultiplied[3];
    let straight = |c: f64| if a > 0.0 { c / a } else { 0.0 };
    [
        straight(premultiplied[0]) * opacity,
        straight(premultiplied[1]) * opacity,
        straight(premultiplied[2]) * opacity,
        opacity,
    ]
}

enum SourceContent<'a> {
    /// Premultiplied working-space color, opacity applied.
    Fill([f64; 4]),
    Raster {
        level: &'a RasterLevel,
        codec: Codec,
        /// Image space → working space; `None` for the identity (keeps huge values exact).
        matrix: Option<Mat3>,
        opacity: f64,
        /// Where the image is in the document; `level` is level 0 unless resampled.
        placement: Placement,
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

    let ops: Vec<Op> = steps(document)
        .into_iter()
        .filter_map(|step| match step {
            Step::Layer {
                layer,
                mode,
                opacity,
                atop,
                transform,
            } => source(layer, mode, opacity, atop, transform).map(|s| Op::Layer(Box::new(s))),
            Step::Adjust {
                layer,
                adjustment,
                opacity,
                transform,
            } => Some(Op::Adjust {
                adjustment: adjustment.prepare(),
                opacity: f64::from(opacity),
                mask: enabled(layer).and_then(|m| mask_source(m, transform)),
            }),
            Step::Begin { isolated } => Some(Op::Begin { isolated }),
            Step::End {
                mask,
                mask_transform,
                mode,
                opacity,
                isolated,
                atop,
            } => Some(Op::End {
                mode,
                opacity: f64::from(opacity),
                mask: mask.and_then(|m| mask_source(m, mask_transform)),
                isolated,
                atop,
            }),
        })
        .collect();
    let blender = Blender::new(document.blend_space());

    let width = region.width as usize;
    let row_len = width * 4;
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let rows_per_chunk = (region.height as usize).div_ceil(threads).max(1);
    let chunks: Vec<&mut [f32]> = out.chunks_mut(rows_per_chunk * row_len).collect();
    let mut reports = vec![CompositeReport::default(); chunks.len()];
    std::thread::scope(|scope| {
        for (chunk_index, (chunk, report)) in chunks.into_iter().zip(&mut reports).enumerate() {
            let (ops, blender) = (&ops, &blender);
            scope.spawn(move || {
                let mut acc = vec![[0.0f64; 4]; width];
                // One accumulator per open group, reused from row to row.
                let mut stack = Vec::new();
                for (i, row) in chunk.chunks_exact_mut(row_len).enumerate() {
                    // Fits: the row is inside the region, whose bottom fits the document.
                    let y = region.y + (chunk_index * rows_per_chunk + i) as u32;
                    composite_row(ops, blender, region.x, y, &mut acc, &mut stack, report);
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

/// Composite the pixels `x0..x0 + acc.len()` of row `y` into `acc`, with `stack` holding what
/// open groups pushed (grown as needed).
fn composite_row(
    ops: &[Op],
    blender: &Blender,
    x0: u32,
    y: u32,
    acc: &mut [[f64; 4]],
    stack: &mut Vec<Vec<[f64; 4]>>,
    report: &mut CompositeReport,
) {
    let table = weight_table();
    acc.fill([0.0; 4]);
    let mut depth = 0;
    for op in ops {
        let source = match op {
            Op::Begin { isolated } => {
                if stack.len() == depth {
                    stack.push(vec![[0.0; 4]; acc.len()]);
                }
                stack[depth].copy_from_slice(acc);
                depth += 1;
                if *isolated {
                    acc.fill([0.0; 4]);
                }
                continue;
            }
            Op::End {
                mode,
                opacity,
                mask,
                isolated,
                atop,
            } => {
                // Steps are balanced: every end has its begin.
                let Some(below) = depth.checked_sub(1) else {
                    continue;
                };
                depth = below;
                for (i, (dst, below)) in acc.iter_mut().zip(&stack[below]).enumerate() {
                    // Fits: the pixel is inside the region.
                    let x = x0 + i as u32;
                    let coverage = opacity * mask.as_ref().map_or(1.0, |m| m.coverage(x, y, table));
                    if *isolated {
                        let mut src = dst.map(|c| c * coverage);
                        if *mode == BlendMode::Dissolve {
                            src = dissolve(src, x, y);
                        }
                        let mut out = *below;
                        if *atop {
                            blender.blend_atop(*mode, &src, &mut out);
                        } else {
                            blender.blend(*mode, &src, &mut out);
                        }
                        *dst = out;
                    } else {
                        *dst = blender.fade(below, dst, coverage);
                    }
                }
                continue;
            }
            Op::Adjust {
                adjustment,
                opacity,
                mask,
            } => {
                for (i, dst) in acc.iter_mut().enumerate() {
                    // Fits: the pixel is inside the region.
                    let x = x0 + i as u32;
                    let coverage = opacity * mask.as_ref().map_or(1.0, |m| m.coverage(x, y, table));
                    *dst = blender.adjust(adjustment, dst, coverage);
                }
                continue;
            }
            Op::Layer(source) => source,
        };
        let mode = source.mode;
        let masked = |src: [f64; 4], x: u32| {
            let src = match &source.mask {
                Some(mask) => {
                    let coverage = mask.coverage(x, y, table);
                    src.map(|c| c * coverage)
                }
                None => src,
            };
            if mode == BlendMode::Dissolve {
                dissolve(src, x, y)
            } else {
                src
            }
        };
        match &source.content {
            SourceContent::Fill(color) => {
                for (i, dst) in acc.iter_mut().enumerate() {
                    // Fits: the pixel is inside the region.
                    let src = masked(*color, x0 + i as u32);
                    if source.atop {
                        blender.blend_atop(mode, &src, dst);
                    } else {
                        blender.blend(mode, &src, dst);
                    }
                }
            }
            SourceContent::Raster {
                level,
                codec,
                matrix,
                opacity,
                placement: Placement::Resampled(r),
            } => {
                let size = level.size();
                let bytes = codec.bytes_per_pixel;
                for (i, dst) in acc.iter_mut().enumerate() {
                    // Fits: the pixel is inside the region.
                    let x = x0 + i as u32;
                    let p = (f64::from(x) + 0.5, f64::from(y) + 0.5);
                    if !r.reaches(p, size.width, size.height) {
                        continue;
                    }
                    let (color, inside) = r.sample(table, p, |i, j| {
                        stored_texel(level, bytes, i, j).map(|px| {
                            px.map_or([0.0; 4], |px| {
                                texel(codec, px, matrix.as_ref(), 1.0, report)
                            })
                        })
                    });
                    let src = if source.replaces_alpha {
                        opaque(color, *opacity).map(|c| c * inside)
                    } else {
                        color.map(|c| c * opacity)
                    };
                    let src = masked(src, x);
                    if source.atop {
                        blender.blend_atop(mode, &src, dst);
                    } else {
                        blender.blend(mode, &src, dst);
                    }
                }
            }
            SourceContent::Raster {
                level,
                codec,
                matrix,
                opacity,
                placement: Placement::Offset(ox, oy),
            } => {
                let size = level.size();
                // The raster sits at its offset and may be smaller: transparent outside. Pixel
                // coordinates below are the image's (the document's minus the offset).
                let ly = i64::from(y) - oy;
                let lx0 = i64::from(x0) - ox;
                if ly < 0 || ly >= i64::from(size.height) {
                    continue;
                }
                let first = lx0.max(0);
                let last = (lx0 + acc.len() as i64).min(i64::from(size.width));
                if first >= last {
                    continue;
                }
                // In range just above.
                let (ly, first, end) = (ly as u32, first as u32, last as u32);
                let (row, local_y) = (ly / TILE_SIZE, (ly % TILE_SIZE) as usize);
                let mut x = first;
                // One tile-wide run at a time.
                while x < end {
                    let col = x / TILE_SIZE;
                    let run_end = end.min((col + 1).saturating_mul(TILE_SIZE));
                    if let Some(tile) = level.tile(TileCoord { col, row }) {
                        for px_x in x..run_end {
                            let local_x = (px_x % TILE_SIZE) as usize;
                            // The document pixel of this texel: inside the region's row.
                            let doc_x = (i64::from(px_x) + ox) as u32;
                            let start =
                                (local_y * TILE_SIZE as usize + local_x) * codec.bytes_per_pixel;
                            let Some(px) = tile.get(start..start + codec.bytes_per_pixel) else {
                                continue;
                            };
                            let src = if source.replaces_alpha {
                                opaque(texel(codec, px, matrix.as_ref(), 1.0, report), *opacity)
                            } else {
                                texel(codec, px, matrix.as_ref(), *opacity, report)
                            };
                            let src = masked(src, doc_x);
                            let dst = &mut acc[(doc_x - x0) as usize];
                            if source.atop {
                                blender.blend_atop(mode, &src, dst);
                            } else {
                                blender.blend(mode, &src, dst);
                            }
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
    use crate::adjust::Adjustment;
    use crate::blend::BlendSpace;
    use crate::color::{
        AlphaMode, ChannelLayout, ColorSpace, LinearRgba, PixelFormat, SampleType,
        TransferFunction, WORKING_SPACE, srgb_decode,
    };
    use crate::convert::{ConversionReport, ConvertOptions, Converter, WHITE_MATTE};
    use crate::document::{Layer, LayerId};
    use crate::edit::Edit;
    use crate::raster::RasterImage;

    /// A document blending in linear space, where normal mode is premultiplied "over".
    fn linear_document(size: Size) -> Document {
        let mut doc = Document::new(size);
        Edit::SetBlendSpace {
            space: BlendSpace::Linear,
        }
        .apply(&mut doc)
        .unwrap();
        doc
    }

    /// Premultiplied "over": `dst = src + dst × (1 − src.a)`.
    fn over(src: &[f64; 4], dst: &mut [f64; 4]) {
        let keep = 1.0 - src[3];
        for (d, s) in dst.iter_mut().zip(src) {
            *d = s + *d * keep;
        }
    }

    fn add(doc: &mut Document, content: LayerContent, opacity: f32, visible: bool) -> LayerId {
        let id = doc.allocate_layer_id();
        let index = doc.layers().len();
        Edit::InsertLayer {
            parent: None,
            index,
            layer: Layer {
                transform: crate::transform::Affine::IDENTITY,
                clipped: false,
                id,
                name: format!("layer {}", id.get()),
                visible,
                opacity,
                blend_mode: BlendMode::Normal,
                mask: None,
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
        let mut doc = linear_document(Size::new(3, 1));
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
        let empty = linear_document(Size::new(2, 2));
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
        let mut doc = linear_document(size);
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
        let mut doc = linear_document(Size::new(2, 1));
        let pixels = [[1e5, -3.0, 0.5, 1.0], [3e38, 1.0, 1.0, 1.0]];
        add(&mut doc, float_raster(Size::new(2, 1), &pixels), 1.0, true);
        let (out, report) = composite(&doc, doc.size().bounds());
        assert_eq!(out, pixels.as_flattened());
        assert_eq!(report.non_finite, 0);
        // The display path still clamps (see MAX_FINITE_SAMPLE); export does not.
    }

    #[test]
    fn non_finite_samples_are_mapped_and_counted() {
        let mut doc = linear_document(Size::new(3, 1));
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
        let mut doc = linear_document(size);
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
            // Irrelevant: the source is opaque or keeps its alpha.
            matte: WHITE_MATTE,
            blend_space: doc.blend_space(),
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

    /// A 2×1 straight RGBA8 sRGB raster: an opaque red pixel and a half-transparent green one.
    fn red_and_half_green() -> Arc<RasterImage> {
        let pixels = [255, 0, 0, 255, 0, 255, 0, 128];
        Arc::new(
            RasterImage::from_pixels(Size::new(2, 1), PixelFormat::RGBA8_SRGB, &pixels).unwrap(),
        )
    }

    fn set_mask(doc: &mut Document, id: LayerId, mask: Option<crate::document::LayerMask>) {
        Edit::SetLayerMask { id, mask }.apply(doc).unwrap();
    }

    #[test]
    fn a_mask_from_transparency_keeps_the_look_and_moves_the_alpha() {
        let image = red_and_half_green();
        let mut doc = linear_document(Size::new(2, 1));
        let id = add(
            &mut doc,
            LayerContent::Raster {
                image: image.clone(),
            },
            1.0,
            true,
        );
        let (before, _) = composite(&doc, doc.size().bounds());

        let mask = crate::document::LayerMask::from_transparency(&image).unwrap();
        assert_eq!(mask.image.format().layout, ChannelLayout::Gray);
        set_mask(&mut doc, id, Some(mask));
        // Same coverage, now from the mask: the look does not change.
        let (masked, _) = composite(&doc, doc.size().bounds());
        assert_close(&masked, &before);

        // Disabled: the layer's own alpha is still ignored, so the green pixel is opaque.
        Edit::SetLayerMaskEnabled { id, enabled: false }
            .apply(&mut doc)
            .unwrap();
        let (disabled, _) = composite(&doc, doc.size().bounds());
        assert!((disabled[7] - 1.0).abs() < 1e-6, "{disabled:?}");
        assert!(disabled[5] > 0.9, "full green: {disabled:?}");

        // Deleted: the layer's own alpha is back.
        set_mask(&mut doc, id, None);
        let (deleted, _) = composite(&doc, doc.size().bounds());
        assert_close(&deleted, &before);
    }

    #[test]
    fn mask_values_scale_coverage_and_outside_the_mask_is_hidden() {
        // A gray 8-bit linear mask of one pixel at 25%: the second pixel is outside it.
        let format = PixelFormat {
            layout: ChannelLayout::Gray,
            sample: SampleType::U8,
            color_space: ColorSpace::LINEAR_SRGB,
            alpha: AlphaMode::Straight,
        };
        let mask_image =
            Arc::new(RasterImage::from_pixels(Size::new(1, 1), format, &[64]).unwrap());
        let mut doc = linear_document(Size::new(2, 1));
        let id = add(&mut doc, fill(0.0, 0.0, 1.0, 1.0), 1.0, true);
        set_mask(
            &mut doc,
            id,
            Some(crate::document::LayerMask {
                image: mask_image,
                enabled: true,
                replaces_alpha: false,
            }),
        );
        let (out, _) = composite(&doc, doc.size().bounds());
        let quarter = 64.0 / 255.0;
        assert_close(&out, &[0.0, 0.0, quarter, quarter, 0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn mask_edits_round_trip_and_are_validated() {
        let image = red_and_half_green();
        let mut doc = linear_document(Size::new(2, 1));
        let id = add(
            &mut doc,
            LayerContent::Raster {
                image: image.clone(),
            },
            1.0,
            true,
        );
        assert!(matches!(
            Edit::SetLayerMaskEnabled { id, enabled: false }.apply(&mut doc),
            Err(crate::edit::EditError::NoMask(_))
        ));
        let mask = crate::document::LayerMask::from_transparency(&image).unwrap();
        let inverse = Edit::SetLayerMask {
            id,
            mask: Some(mask.clone()),
        }
        .apply(&mut doc)
        .unwrap();
        assert_eq!(inverse, Edit::SetLayerMask { id, mask: None });
        let toggled = Edit::SetLayerMaskEnabled { id, enabled: false }
            .apply(&mut doc)
            .unwrap();
        assert_eq!(toggled, Edit::SetLayerMaskEnabled { id, enabled: true });
        // A color image is not a mask.
        let not_gray = crate::document::LayerMask {
            image: image.clone(),
            enabled: true,
            replaces_alpha: false,
        };
        assert_eq!(
            Edit::SetLayerMask {
                id,
                mask: Some(not_gray)
            }
            .apply(&mut doc),
            Err(crate::edit::EditError::InvalidMask)
        );
        // An image without alpha has no transparency to take.
        let rgb = PixelFormat {
            layout: ChannelLayout::Rgb,
            ..PixelFormat::RGBA8_SRGB
        };
        let opaque_image = RasterImage::from_pixels(Size::new(1, 1), rgb, &[1, 2, 3]).unwrap();
        assert!(crate::document::LayerMask::from_transparency(&opaque_image).is_none());
    }

    #[test]
    fn alpha_masks_keep_the_sample_type_and_every_value() {
        let size = Size::new(300, 260);
        let format = PixelFormat {
            layout: ChannelLayout::Rgba,
            sample: SampleType::U16,
            color_space: ColorSpace::SRGB,
            alpha: AlphaMode::Straight,
        };
        let alpha = |x: u32, y: u32| ((x * 211 + y * 17) % 65536) as u16;
        let pixels: Vec<u8> = (0..size.height)
            .flat_map(|y| (0..size.width).map(move |x| (x, y)))
            .flat_map(|(x, y)| [1000u16, 2000, 3000, alpha(x, y)])
            .flat_map(u16::to_ne_bytes)
            .collect();
        let image = RasterImage::from_pixels(size, format, &pixels).unwrap();
        let mask = image.alpha_mask().unwrap();
        assert_eq!(mask.size(), size);
        assert_eq!(mask.format().sample, SampleType::U16);
        assert_eq!(mask.format().layout, ChannelLayout::Gray);
        let level = &mask.levels()[0];
        for (x, y) in [(0, 0), (299, 259), (256, 3), (17, 255)] {
            let tile = level
                .tile(TileCoord {
                    col: x / TILE_SIZE,
                    row: y / TILE_SIZE,
                })
                .unwrap();
            let at = (((y % TILE_SIZE) * TILE_SIZE + x % TILE_SIZE) * 2) as usize;
            assert_eq!(u16::from_ne_bytes([tile[at], tile[at + 1]]), alpha(x, y));
        }
    }

    /// A layer not yet in a document (its id allocated from `doc`).
    fn new_layer(
        doc: &mut Document,
        content: LayerContent,
        mode: BlendMode,
        opacity: f32,
    ) -> Layer {
        let id = doc.allocate_layer_id();
        Layer {
            transform: crate::transform::Affine::IDENTITY,
            clipped: false,
            id,
            name: format!("layer {}", id.get()),
            visible: true,
            opacity,
            blend_mode: mode,
            mask: None,
            content,
        }
    }

    /// Insert `layer` at the top of `doc`.
    fn push(doc: &mut Document, layer: Layer) {
        let index = doc.layers().len();
        Edit::InsertLayer {
            parent: None,
            index,
            layer,
        }
        .apply(doc)
        .unwrap();
    }

    fn group(doc: &mut Document, children: Vec<Layer>, pass_through: bool) -> Layer {
        new_layer(
            doc,
            LayerContent::Group {
                children,
                pass_through,
            },
            BlendMode::Normal,
            1.0,
        )
    }

    fn document(space: BlendSpace) -> Document {
        let mut doc = Document::new(Size::new(4, 2));
        Edit::SetBlendSpace { space }.apply(&mut doc).unwrap();
        doc
    }

    /// Varied straight colors and coverages.
    fn varied(seed: f32) -> LayerContent {
        let pixels: Vec<[f32; 4]> = (0..8)
            .map(|i| {
                let t = i as f32 / 8.0;
                [
                    (t + seed).fract(),
                    (0.3 + t * seed).fract(),
                    (0.7 - t * 0.5 + seed * 0.1).fract(),
                    0.25 + 0.75 * ((t * 3.0 + seed).fract()),
                ]
            })
            .collect();
        float_raster(Size::new(4, 2), &pixels)
    }

    fn opaque_base(doc: &mut Document) {
        let base = new_layer(doc, fill(0.3, 0.5, 0.2, 1.0), BlendMode::Normal, 1.0);
        push(doc, base);
    }

    fn all(doc: &Document) -> Vec<f32> {
        composite(doc, doc.size().bounds()).0
    }

    #[test]
    fn pass_through_groups_at_full_opacity_are_their_children() {
        for space in [BlendSpace::Perceptual, BlendSpace::Linear] {
            let mut flat = document(space);
            opaque_base(&mut flat);
            let a = new_layer(&mut flat, varied(0.1), BlendMode::Multiply, 0.7);
            let b = new_layer(&mut flat, fill(0.9, 0.2, 0.4, 0.8), BlendMode::Screen, 0.5);
            push(&mut flat, a.clone());
            push(&mut flat, b.clone());

            let mut grouped = document(space);
            opaque_base(&mut grouped);
            // The children keep their ids: allocate them in this document too.
            while grouped.next_layer_id() <= flat.next_layer_id() {
                grouped.allocate_layer_id();
            }
            let g = group(&mut grouped, vec![a, b], true);
            push(&mut grouped, g);
            assert!(
                steps(&grouped)
                    .iter()
                    .all(|s| matches!(s, Step::Layer { .. })),
                "a neutral pass-through group is inlined"
            );
            assert_close(&all(&grouped), &all(&flat));
        }
    }

    #[test]
    fn pass_through_opacity_and_mask_fade_against_what_is_below() {
        for space in [BlendSpace::Perceptual, BlendSpace::Linear] {
            let mut below = document(space);
            opaque_base(&mut below);
            let mut with = below.clone();
            let child = new_layer(&mut with, varied(0.4), BlendMode::Overlay, 0.9);
            push(&mut with, child.clone());

            let mut grouped = below.clone();
            while grouped.next_layer_id() <= with.next_layer_id() {
                grouped.allocate_layer_id();
            }
            let mut g = group(&mut grouped, vec![child], true);
            g.opacity = 0.6;
            // A mask: coverage x / 4 along each row.
            let mask: Vec<u8> = (0..8)
                .flat_map(|i| ((i % 4) as f32 / 4.0).to_ne_bytes())
                .collect();
            let format = PixelFormat {
                layout: ChannelLayout::Gray,
                sample: SampleType::F32,
                color_space: ColorSpace::LINEAR_SRGB,
                alpha: AlphaMode::Straight,
            };
            g.mask = Some(crate::document::LayerMask {
                image: Arc::new(RasterImage::from_pixels(Size::new(4, 2), format, &mask).unwrap()),
                enabled: true,
                replaces_alpha: false,
            });
            push(&mut grouped, g);

            let blender = Blender::new(space);
            let (b, w) = (all(&below), all(&with));
            let expected: Vec<f32> = b
                .as_chunks::<4>()
                .0
                .iter()
                .zip(w.as_chunks::<4>().0)
                .enumerate()
                .flat_map(|(i, (b, w))| {
                    let t = 0.6 * (i % 4) as f64 / 4.0;
                    let b: [f64; 4] = std::array::from_fn(|k| f64::from(b[k]));
                    let w: [f64; 4] = std::array::from_fn(|k| f64::from(w[k]));
                    blender.fade(&b, &w, t).map(|v| v as f32)
                })
                .collect();
            assert_close(&all(&grouped), &expected);
        }
    }

    #[test]
    fn isolated_groups_blend_their_result_as_one_layer() {
        for space in [BlendSpace::Perceptual, BlendSpace::Linear] {
            // One normal child: the group's result is the child itself.
            let mut flat = document(space);
            opaque_base(&mut flat);
            let child = new_layer(&mut flat, varied(0.7), BlendMode::Normal, 0.8);
            let mut alone = child.clone();
            alone.blend_mode = BlendMode::Multiply;
            alone.opacity = 0.8 * 0.5;
            push(&mut flat, alone);

            let mut grouped = document(space);
            opaque_base(&mut grouped);
            while grouped.next_layer_id() <= flat.next_layer_id() {
                grouped.allocate_layer_id();
            }
            let mut g = group(&mut grouped, vec![child], false);
            g.blend_mode = BlendMode::Multiply;
            g.opacity = 0.5;
            push(&mut grouped, g);
            assert!(matches!(steps(&grouped)[1], Step::Begin { isolated: true }));
            assert_close(&all(&grouped), &all(&flat));
        }
    }

    #[test]
    fn hidden_and_empty_groups_change_nothing() {
        let mut doc = document(BlendSpace::Perceptual);
        opaque_base(&mut doc);
        let reference = all(&doc);
        let child = new_layer(&mut doc, varied(0.2), BlendMode::Difference, 1.0);
        let mut hidden = group(&mut doc, vec![child], false);
        hidden.visible = false;
        push(&mut doc, hidden);
        let mut hidden_child = new_layer(&mut doc, varied(0.3), BlendMode::Normal, 1.0);
        hidden_child.visible = false;
        let mut isolated = group(&mut doc, vec![hidden_child], false);
        isolated.opacity = 0.5;
        push(&mut doc, isolated);
        assert_eq!(steps(&doc).len(), 1, "only the base layer");
        assert_close(&all(&doc), &reference);
    }

    /// A raster whose alpha is 0 or 1 (a shape), with varied colors.
    fn shape(seed: f32) -> LayerContent {
        let pixels: Vec<[f32; 4]> = (0..8)
            .map(|i| {
                let t = i as f32 / 8.0;
                let inside = (i * 3 + seed as usize) % 5 < 3;
                [
                    (t + seed).fract(),
                    (0.2 + t).fract(),
                    (0.9 - t * seed).fract(),
                    if inside { 1.0 } else { 0.0 },
                ]
            })
            .collect();
        float_raster(Size::new(4, 2), &pixels)
    }

    /// The alpha of a raster content as a mask.
    fn alpha_mask_of(content: &LayerContent) -> crate::document::LayerMask {
        let LayerContent::Raster { image } = content else {
            panic!("a raster expected");
        };
        let mut mask = crate::document::LayerMask::from_transparency(image).unwrap();
        mask.replaces_alpha = false;
        mask
    }

    #[test]
    fn clipped_layers_show_only_where_their_base_is() {
        for space in [BlendSpace::Perceptual, BlendSpace::Linear] {
            for mode in [BlendMode::Normal, BlendMode::Multiply, BlendMode::Screen] {
                // With a base of full or no coverage, clipping is the base's alpha as a mask.
                let base = shape(0.3);
                let mut clipped_doc = document(space);
                let b = new_layer(&mut clipped_doc, base.clone(), BlendMode::Normal, 1.0);
                push(&mut clipped_doc, b);
                let mut c = new_layer(&mut clipped_doc, varied(0.6), mode, 0.8);
                c.clipped = true;
                push(&mut clipped_doc, c);

                let mut masked_doc = document(space);
                let b = new_layer(&mut masked_doc, base.clone(), BlendMode::Normal, 1.0);
                push(&mut masked_doc, b);
                let mut c = new_layer(&mut masked_doc, varied(0.6), mode, 0.8);
                c.mask = Some(alpha_mask_of(&base));
                push(&mut masked_doc, c);
                assert_close(&all(&clipped_doc), &all(&masked_doc));
            }
        }
    }

    #[test]
    fn the_base_mode_and_opacity_apply_to_its_clipping_group() {
        for space in [BlendSpace::Perceptual, BlendSpace::Linear] {
            let base = shape(0.7);
            // A base in multiply at 50 % with a clipped layer, over a background...
            let mut clipped_doc = document(space);
            opaque_base(&mut clipped_doc);
            let b = new_layer(&mut clipped_doc, base.clone(), BlendMode::Multiply, 0.5);
            push(&mut clipped_doc, b);
            let mut c = new_layer(&mut clipped_doc, varied(0.1), BlendMode::Overlay, 0.9);
            c.clipped = true;
            push(&mut clipped_doc, c);

            // ... is an isolated group in multiply at 50 % of the base (normal, full opacity)
            // and the layer limited to the base's shape.
            let mut grouped = document(space);
            opaque_base(&mut grouped);
            let b = new_layer(&mut grouped, base.clone(), BlendMode::Normal, 1.0);
            let mut c = new_layer(&mut grouped, varied(0.1), BlendMode::Overlay, 0.9);
            c.mask = Some(alpha_mask_of(&base));
            let mut g = group(&mut grouped, vec![b, c], false);
            g.blend_mode = BlendMode::Multiply;
            g.opacity = 0.5;
            push(&mut grouped, g);
            assert_close(&all(&clipped_doc), &all(&grouped));
        }
    }

    #[test]
    fn a_hidden_base_hides_its_clipping_group_and_a_lone_clipped_layer_draws() {
        let mut doc = document(BlendSpace::Perceptual);
        // Clipped with nothing below it among its siblings: drawn as usual.
        let mut lone = new_layer(&mut doc, varied(0.2), BlendMode::Normal, 1.0);
        lone.clipped = true;
        push(&mut doc, lone);
        let reference = all(&doc);
        let mut base = new_layer(&mut doc, shape(0.4), BlendMode::Normal, 1.0);
        base.visible = false;
        push(&mut doc, base);
        let mut clipped = new_layer(&mut doc, varied(0.5), BlendMode::Normal, 1.0);
        clipped.clipped = true;
        push(&mut doc, clipped);
        assert_eq!(steps(&doc).len(), 1);
        assert_close(&all(&doc), &reference);
    }

    #[test]
    fn atop_keeps_the_coverage_below() {
        for space in [BlendSpace::Perceptual, BlendSpace::Linear] {
            let blender = Blender::new(space);
            let src = [0.3, 0.1, 0.05, 0.6];
            for mode in [BlendMode::Normal, BlendMode::Multiply, BlendMode::Hue] {
                let mut dst = [0.2, 0.2, 0.1, 0.4];
                blender.blend_atop(mode, &src, &mut dst);
                assert!((dst[3] - 0.4).abs() < 1e-12, "{mode:?}: {dst:?}");
                // Over nothing: nothing.
                let mut empty = [0.0; 4];
                blender.blend_atop(mode, &src, &mut empty);
                assert_eq!(empty, [0.0; 4]);
                // Over an opaque backdrop: the usual blend.
                let (mut a, mut b) = ([0.2, 0.3, 0.4, 1.0], [0.2, 0.3, 0.4, 1.0]);
                blender.blend_atop(mode, &src, &mut a);
                blender.blend(mode, &src, &mut b);
                for k in 0..4 {
                    assert!((a[k] - b[k]).abs() < 1e-12, "{mode:?} {space:?}");
                }
            }
        }
    }

    #[test]
    fn a_moved_layer_is_its_image_placed_there() {
        for space in [BlendSpace::Perceptual, BlendSpace::Linear] {
            for (dx, dy) in [(1i64, 1i64), (-2, 1), (3, -1), (5, 0)] {
                // A 3 × 2 raster with a mask, moved in a 4 × 2 document...
                let pixels: Vec<[f32; 4]> = (0..6)
                    .map(|i| {
                        [
                            i as f32 / 6.0,
                            0.5,
                            1.0 - i as f32 / 6.0,
                            0.4 + i as f32 / 10.0,
                        ]
                    })
                    .collect();
                let small = float_raster(Size::new(3, 2), &pixels);
                let mask_values: Vec<u8> = (0..6)
                    .flat_map(|i| (i as f32 / 5.0).to_ne_bytes())
                    .collect();
                let gray = PixelFormat {
                    layout: ChannelLayout::Gray,
                    sample: SampleType::F32,
                    color_space: ColorSpace::LINEAR_SRGB,
                    alpha: AlphaMode::Straight,
                };
                let mask = crate::document::LayerMask {
                    image: Arc::new(
                        RasterImage::from_pixels(Size::new(3, 2), gray, &mask_values).unwrap(),
                    ),
                    enabled: true,
                    replaces_alpha: false,
                };
                let mut moved = document(space);
                opaque_base(&mut moved);
                let mut layer = new_layer(&mut moved, small, BlendMode::Screen, 0.9);
                layer.mask = Some(mask.clone());
                layer.transform = Affine::translation(dx as f64, dy as f64);
                push(&mut moved, layer);

                // ... is the same image and mask placed there on a document-sized raster.
                let canvas = Size::new(4, 2);
                let place = |bytes: &[u8], bpp: usize, background: &[u8], format| {
                    // The part of the 3 × 2 image inside the canvas, and where it lands.
                    let x0 = dx.max(0);
                    let x1 = (dx + 3).min(4);
                    let y0 = dy.max(0);
                    let y1 = (dy + 2).min(2);
                    let (w, h) = ((x1 - x0).max(0) as u32, (y1 - y0).max(0) as u32);
                    let mut part = Vec::new();
                    for y in y0..y0 + i64::from(h) {
                        for x in x0..x0 + i64::from(w) {
                            let (sx, sy) = ((x - dx) as usize, (y - dy) as usize);
                            part.extend(&bytes[(sy * 3 + sx) * bpp..][..bpp]);
                        }
                    }
                    let rect = if w == 0 || h == 0 {
                        Rect::new(0, 0, 0, 0)
                    } else {
                        Rect::new(x0 as u32, y0 as u32, w, h)
                    };
                    RasterImage::from_placed(canvas, format, rect, &part, background).unwrap()
                };
                let rgba: Vec<u8> = pixels
                    .iter()
                    .flatten()
                    .flat_map(|v| v.to_ne_bytes())
                    .collect();
                let rgba_format = PixelFormat {
                    layout: ChannelLayout::Rgba,
                    sample: SampleType::F32,
                    color_space: WORKING_SPACE,
                    alpha: AlphaMode::Straight,
                };
                let placed_image = place(&rgba, 16, &[0; 16], rgba_format);
                let placed_mask = place(&mask_values, 4, &[0; 4], gray);
                let mut reference = document(space);
                opaque_base(&mut reference);
                let mut layer = new_layer(
                    &mut reference,
                    LayerContent::Raster {
                        image: Arc::new(placed_image),
                    },
                    BlendMode::Screen,
                    0.9,
                );
                layer.mask = Some(crate::document::LayerMask {
                    image: Arc::new(placed_mask),
                    ..mask
                });
                push(&mut reference, layer);
                assert_close(&all(&moved), &all(&reference));
            }
        }
    }

    #[test]
    fn a_group_moves_its_layers() {
        let mut doc = document(BlendSpace::Linear);
        let inner = new_layer(&mut doc, varied(0.3), BlendMode::Normal, 1.0);
        let mut g = group(&mut doc, vec![inner.clone()], false);
        g.transform = Affine::translation(2.0, 1.0);
        push(&mut doc, g);
        let mut flat = document(BlendSpace::Linear);
        while flat.next_layer_id() <= doc.next_layer_id() {
            flat.allocate_layer_id();
        }
        let mut moved = inner;
        moved.transform = Affine::translation(2.0, 1.0);
        push(&mut flat, moved);
        assert_close(&all(&doc), &all(&flat));
    }

    #[test]
    fn a_quarter_turned_layer_is_its_image_turned_exactly() {
        let pixels: Vec<[f32; 4]> = (0..6)
            .map(|i| {
                [
                    i as f32 / 6.0,
                    0.25,
                    1.0 - i as f32 / 7.0,
                    0.3 + i as f32 / 9.0,
                ]
            })
            .collect();
        // (x, y) ↦ (2 − y, x): the 3 × 2 image covers x ∈ [0, 2), y ∈ [0, 3).
        let turn = Affine {
            a: 0.0,
            b: 1.0,
            c: -1.0,
            d: 0.0,
            e: 2.0,
            f: 0.0,
        };
        let mut turned = Document::new(Size::new(4, 4));
        let mut layer = new_layer(
            &mut turned,
            float_raster(Size::new(3, 2), &pixels),
            BlendMode::Normal,
            1.0,
        );
        layer.transform = turn;
        push(&mut turned, layer);
        // Document pixel (x, y) shows image pixel (y, 1 − x): a 2 × 3 image, not resampled.
        let rotated: Vec<[f32; 4]> = (0..3)
            .flat_map(|y| (0..2).map(move |x| (y, x)))
            .map(|(y, x)| pixels[(1 - x) * 3 + y])
            .collect();
        let mut reference = Document::new(Size::new(4, 4));
        let layer = new_layer(
            &mut reference,
            float_raster(Size::new(2, 3), &rotated),
            BlendMode::Normal,
            1.0,
        );
        push(&mut reference, layer);
        assert_eq!(all(&turned), all(&reference));
    }

    #[test]
    fn a_scaled_layer_keeps_its_flat_areas_and_fades_at_its_edges() {
        let color = [0.2, 0.5, 0.7, 1.0];
        let mut doc = Document::new(Size::new(24, 20));
        let mut layer = new_layer(
            &mut doc,
            float_raster(Size::new(4, 4), &[color; 16]),
            BlendMode::Normal,
            1.0,
        );
        // 12 × 12 pixels from (2.5, 1).
        layer.transform = Affine::scale(3.0, 3.0).then(Affine::translation(2.5, 1.0));
        push(&mut doc, layer);
        let out = all(&doc);
        let at = |x: usize, y: usize| &out[(y * 24 + x) * 4..][..4];
        for (a, e) in at(8, 7).iter().zip(color) {
            assert!((a - e).abs() < 1e-6, "{:?}", at(8, 7));
        }
        assert_eq!(at(20, 18), [0.0; 4]);
        // The left edge runs through pixel 2: half covered.
        let edge = at(2, 7)[3];
        assert!(edge > 0.2 && edge < 0.8, "{edge}");
    }

    fn adjustment(adjustment: Adjustment) -> LayerContent {
        LayerContent::Adjustment { adjustment }
    }

    const WARM: Adjustment = Adjustment::HueSaturation {
        hue: 40.0,
        saturation: 30.0,
        lightness: -10.0,
    };

    #[test]
    fn an_adjustment_changes_what_is_below_mixed_by_its_opacity() {
        for space in [BlendSpace::Perceptual, BlendSpace::Linear] {
            for (adjust, opacity) in [
                (WARM, 1.0),
                (
                    Adjustment::Exposure {
                        exposure: 1.0,
                        offset: 0.02,
                        gamma: 1.3,
                    },
                    0.6,
                ),
                (
                    Adjustment::Levels {
                        input_black: 0.1,
                        input_white: 0.8,
                        gamma: 0.7,
                        output_black: 0.05,
                        output_white: 0.95,
                    },
                    0.3,
                ),
            ] {
                let mut doc = document(space);
                let layer = new_layer(&mut doc, varied(0.2), BlendMode::Normal, 1.0);
                push(&mut doc, layer);
                let below = all(&doc);
                let layer = new_layer(&mut doc, adjustment(adjust), BlendMode::Normal, opacity);
                push(&mut doc, layer);
                let blender = Blender::new(space);
                let expected: Vec<f32> = below
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .flat_map(|px| {
                        let px = px.map(f64::from);
                        blender
                            .adjust(&adjust.prepare(), &px, f64::from(opacity))
                            .map(|v| v as f32)
                    })
                    .collect();
                assert_close(&all(&doc), &expected);
            }
        }
    }

    #[test]
    fn an_adjustment_in_an_isolated_group_or_clipped_changes_only_its_own() {
        for space in [BlendSpace::Perceptual, BlendSpace::Linear] {
            // Below everything, an opaque background; above, a shape with transparent pixels.
            let mut base_only = document(space);
            opaque_base(&mut base_only);
            let shape_layer = new_layer(&mut base_only, shape(0.5), BlendMode::Normal, 1.0);
            push(&mut base_only, shape_layer.clone());
            let without = all(&base_only);

            // In an isolated group with the shape, or clipped to it: where the shape is fully
            // transparent, the background is untouched.
            let mut grouped = document(space);
            opaque_base(&mut grouped);
            let s = new_layer(&mut grouped, shape(0.5), BlendMode::Normal, 1.0);
            let a = new_layer(&mut grouped, adjustment(WARM), BlendMode::Normal, 1.0);
            let g = group(&mut grouped, vec![s, a], false);
            push(&mut grouped, g);

            let mut clipped = document(space);
            opaque_base(&mut clipped);
            let s = new_layer(&mut clipped, shape(0.5), BlendMode::Normal, 1.0);
            push(&mut clipped, s);
            let mut a = new_layer(&mut clipped, adjustment(WARM), BlendMode::Normal, 1.0);
            a.clipped = true;
            push(&mut clipped, a);

            // In a pass-through group, the background changes too.
            let mut passing = document(space);
            opaque_base(&mut passing);
            let s = new_layer(&mut passing, shape(0.5), BlendMode::Normal, 1.0);
            let a = new_layer(&mut passing, adjustment(WARM), BlendMode::Normal, 1.0);
            let g = group(&mut passing, vec![s, a], true);
            push(&mut passing, g);

            let alphas = match &shape_layer.content {
                LayerContent::Raster { image } => (0..8)
                    .map(|i| image.alpha_at(i % 4, i / 4))
                    .collect::<Vec<_>>(),
                _ => unreachable!("a raster"),
            };
            let (grouped, clipped, passing) = (all(&grouped), all(&clipped), all(&passing));
            let mut seen_transparent = false;
            for (i, alpha) in alphas.iter().enumerate() {
                let px = |v: &[f32]| v[i * 4..i * 4 + 4].to_vec();
                if *alpha == 0.0 {
                    seen_transparent = true;
                    assert_close(&px(&grouped), &px(&without));
                    assert_close(&px(&clipped), &px(&without));
                    assert_ne!(px(&passing), px(&without));
                }
            }
            assert!(seen_transparent, "the shape has transparent pixels");
            assert_close(&grouped, &clipped);
        }
    }
}
