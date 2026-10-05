//! Filters on the GPU (ADR 0035): the looks at filtered layers (`stack::LookJob`) the display
//! asks for, computed by compute passes (filter.wgsl) on a thread of their own and read back as
//! the look's image: Gaussian Blur, and Unsharp Mask and High Pass made from it. Only the common
//! case for now: 8-bit RGB or RGBA sRGB layers in a perceptual document, steps without a selection, a
//! reach of at most [`MAX_REACH`] pixels; the CPU computes the others (`LookJob::run`).

use std::sync::mpsc;

use slopshop_core::blend::BlendSpace;
use slopshop_core::color::{ChannelLayout, PixelFormat};
use slopshop_core::filter::{CLARITY_STRENGTH, Filter, LINE_UP_TO, MEDIAN_UP_TO, line_offsets};
use slopshop_core::raster::{RasterImage, TILE_SIZE};
use slopshop_core::stack::{FilterStep, LookJob};
use wgpu::util::DeviceExt;

/// The farthest a pixel's result reads, in pixels on each side.
const MAX_REACH: u32 = 1024;

/// Pixels a side of a workgroup (filter.wgsl).
const WORKGROUP: u32 = 16;

/// The GPU's filter pipelines.
#[derive(Debug)]
pub(crate) struct GpuFilter {
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    rows: wgpu::ComputePipeline,
    columns: wgpu::ComputePipeline,
    line: wgpu::ComputePipeline,
    noise: wgpu::ComputePipeline,
    median: wgpu::ComputePipeline,
    /// Buffers of looks computed, kept for the next ones of the same size (a slider dragged
    /// over a filter asks for the same crop at each setting).
    kept: std::sync::Mutex<Vec<LookBuffers>>,
}

/// The buffers a look of `width` × `height` pixels is computed in.
#[derive(Debug)]
struct LookBuffers {
    width: u32,
    height: u32,
    /// Whether `rows` has room for a blur kept between two passes.
    keeps: bool,
    packed: wgpu::Buffer,
    rows: wgpu::Buffer,
    output: wgpu::Buffer,
    readback: wgpu::Buffer,
}

/// How many sizes of look buffers are kept (one or two layers' looks at once).
const KEPT_LOOKS: usize = 2;

impl GpuFilter {
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("filter"),
            source: wgpu::ShaderSource::Wgsl(include_str!("filter.wgsl").into()),
        });
        let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("filter"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                storage(1, true),
                storage(2, true),
                storage(3, false),
                storage(4, false),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("filter"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry_point| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry_point),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry_point),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        };
        Self {
            device: device.clone(),
            queue: queue.clone(),
            layout,
            rows: pipeline("rows_main"),
            columns: pipeline("columns_main"),
            line: pipeline("line_main"),
            noise: pipeline("noise_main"),
            median: pipeline("median_main"),
            kept: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// The filtered crop of `job`, or `None` when the GPU does not take it (another format, a
    /// selection, too far a reach, a GPU error): the CPU computes it then.
    pub(crate) fn look(&self, job: &LookJob) -> Option<RasterImage> {
        // A Liquify look is warped on the CPU (ADR 0037).
        if job.warp.is_some() {
            return None;
        }
        // RGB is stored as RGBA with an opaque alpha, which every filter keeps opaque: the same
        // pixels for the shader (most layers opened from a JPEG are RGB).
        let stored = PixelFormat {
            layout: ChannelLayout::Rgba,
            ..job.format
        };
        if !matches!(job.format.layout, ChannelLayout::Rgb | ChannelLayout::Rgba)
            || stored != PixelFormat::RGBA8_SRGB
        {
            return None;
        }
        let kernels: Vec<Pass> = job
            .steps
            .iter()
            .map(|step| {
                if step.selection.is_some() || step.space != BlendSpace::Perceptual {
                    return None;
                }
                Pass::of(step)
            })
            .collect::<Option<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect();
        let (width, height) = (job.size.width, job.size.height);
        let pixels = u64::from(width) * u64::from(height);
        let limits = self.device.limits();
        // The rows' blur in f32, twice when a blur is kept between two passes.
        let scratch = if kernels.iter().any(|pass| pass.mode == KEEP) {
            32
        } else {
            16
        };
        if pixels == 0
            || pixels * scratch > limits.max_storage_buffer_binding_size
            || pixels * scratch > limits.max_buffer_size
        {
            return None;
        }
        let keeps = kernels.iter().any(|pass| pass.mode == KEEP);
        let scopes = [
            wgpu::ErrorFilter::Internal,
            wgpu::ErrorFilter::Validation,
            wgpu::ErrorFilter::OutOfMemory,
        ]
        .map(|filter| self.device.push_error_scope(filter));
        let buffers = self.buffers(width, height, keeps);
        let result = self.blur(job, &kernels, &buffers);
        let mut failed = false;
        for scope in scopes.into_iter().rev() {
            failed |= pollster::block_on(scope.pop()).is_some();
        }
        if failed {
            return None;
        }
        self.keep(buffers);
        // Level 0 only: a look is shown at the level it is made for.
        RasterImage::from_level0_tiles_only(job.size, job.format, result?).ok()
    }

    /// Buffers for a look of `width` × `height`: those of an earlier look of that size, else
    /// new ones.
    fn buffers(&self, width: u32, height: u32, keeps: bool) -> LookBuffers {
        let mut kept = self
            .kept
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(at) = kept
            .iter()
            .position(|b| b.width == width && b.height == height && (b.keeps || !keeps))
        {
            return kept.swap_remove(at);
        }
        drop(kept);
        let bytes = u64::from(width) * u64::from(height) * 4;
        let buffer = |label, size, usage| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        use wgpu::BufferUsages as U;
        LookBuffers {
            width,
            height,
            keeps,
            packed: buffer("filter input", bytes, U::STORAGE | U::COPY_DST),
            // Twice as large when a blur is kept between two passes (in its second half).
            rows: buffer(
                "filter rows",
                bytes * 4 * if keeps { 2 } else { 1 },
                U::STORAGE,
            ),
            output: buffer("filter output", bytes, U::STORAGE | U::COPY_SRC),
            readback: buffer("filter readback", bytes, U::MAP_READ | U::COPY_DST),
        }
    }

    /// Keep `buffers` for the next looks, the most recent sizes only.
    fn keep(&self, buffers: LookBuffers) {
        let mut kept = self
            .kept
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if kept.len() >= KEPT_LOOKS {
            kept.remove(0);
        }
        kept.push(buffers);
    }

    /// The crop's pixels filtered by each pass in turn, as the look's tiles.
    fn blur(
        &self,
        job: &LookJob,
        kernels: &[Pass],
        buffers: &LookBuffers,
    ) -> Option<Vec<std::sync::Arc<[u8]>>> {
        let (width, height) = (buffers.width, buffers.height);
        let LookBuffers {
            packed,
            rows,
            output,
            readback,
            ..
        } = buffers;
        let bytes = u64::from(width) * u64::from(height) * 4;
        let device = &self.device;
        self.queue
            .write_buffer(packed, 0, &rows_of(job, width, height));
        let groups = (width.div_ceil(WORKGROUP), height.div_ceil(WORKGROUP));
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("filter"),
        });
        // Whether the output holds a step's result the next one filters.
        let mut fresh = false;
        for pass in kernels {
            if fresh {
                encoder.copy_buffer_to_buffer(output, 0, packed, 0, bytes);
                fresh = false;
            }
            fresh |= pass.mode != KEEP;
            let reach = pass.reach;
            let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("filter params"),
                contents: &[width, height, reach, pass.mode]
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .chain(pass.amount.to_le_bytes())
                    .chain(pass.threshold.to_le_bytes())
                    .chain(pass.seed.to_le_bytes())
                    .chain(pass.flags.to_le_bytes())
                    .collect::<Vec<u8>>(),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let weights = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("filter weights"),
                contents: &pass
                    .weights
                    .iter()
                    .flat_map(|w| w.to_le_bytes())
                    .collect::<Vec<u8>>(),
                usage: wgpu::BufferUsages::STORAGE,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("filter"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: params.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: weights.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: packed.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: rows.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: output.as_entire_binding(),
                    },
                ],
            });
            let pipelines = match pass.kind {
                Kind::Separable => vec![&self.rows, &self.columns],
                Kind::Line => vec![&self.line],
                Kind::Noise => vec![&self.noise],
                Kind::Median => vec![&self.median],
            };
            for pipeline in pipelines {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("filter"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &bind_group, &[]);
                pass.dispatch_workgroups(groups.0, groups.1, 1);
            }
        }
        encoder.copy_buffer_to_buffer(output, 0, readback, 0, bytes);
        self.queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            // The receiver only disappears if we already returned; nothing to report then.
            let _ = tx.send(result);
        });
        crate::wait_mapped(device, &rx).ok()?;
        // The tiles straight from the mapped rows.
        let tiles = tiles_of(&slice.get_mapped_range().ok()?, width, height);
        readback.unmap();
        Some(tiles)
    }
}

/// What a pass runs (filter.wgsl's entry points).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A Gaussian along rows then columns, each pixel then made from it (`mode`).
    Separable,
    /// Motion Blur's line: `weights` holds its samples' offsets.
    Line,
    /// Add Noise: `weights` holds the crop's map to the document.
    Noise,
    /// Dust & Scratches' median of the square of `reach`.
    Median,
}

/// The mode of a separable pass whose blur is kept for the next one (filter.wgsl).
const KEEP: u32 = 4;

/// A pass as the shader runs it (`Params` in filter.wgsl); a step is one or two.
struct Pass {
    kind: Kind,
    weights: Vec<f32>,
    /// Half a kernel's width, a line's samples, or a median's radius.
    reach: u32,
    mode: u32,
    amount: f32,
    threshold: f32,
    seed: u32,
    flags: u32,
}

impl Pass {
    /// A Gaussian of standard deviation `sigma`, each pixel then made from it by `mode`.
    fn separable(sigma: f32, mode: u32) -> Option<Self> {
        let weights = gaussian(f64::from(sigma))?;
        Some(Self {
            kind: Kind::Separable,
            reach: (weights.len() / 2) as u32,
            weights,
            mode,
            amount: 0.0,
            threshold: 0.0,
            seed: 0,
            flags: 0,
        })
    }

    /// `step`'s passes, or `None` when it reaches too far (the CPU reduces the layer then).
    fn of(step: &FilterStep) -> Option<Vec<Self>> {
        let none = Self {
            kind: Kind::Separable,
            weights: vec![0.0],
            reach: 0,
            mode: 0,
            amount: 0.0,
            threshold: 0.0,
            seed: 0,
            flags: 0,
        };
        let filter = step.filter;
        match filter {
            Filter::MotionBlur { angle, distance } => {
                if f64::from(distance) > LINE_UP_TO {
                    return None;
                }
                let weights: Vec<f32> = line_offsets(f64::from(angle), f64::from(distance))
                    .into_iter()
                    .flatten()
                    .map(|v| v as f32)
                    .collect();
                return Some(vec![Self {
                    kind: Kind::Line,
                    reach: (weights.len() / 2) as u32,
                    weights,
                    ..none
                }]);
            }
            Filter::AddNoise {
                amount,
                gaussian,
                monochromatic,
                seed,
            } => {
                // In perspective: the CPU places the noise (ADR 0038).
                let map = step.to_document.as_affine()?;
                return Some(vec![Self {
                    kind: Kind::Noise,
                    weights: [map.a, map.b, map.c, map.d, map.e, map.f]
                        .map(|v| v as f32)
                        .to_vec(),
                    amount,
                    seed,
                    flags: u32::from(gaussian) | u32::from(monochromatic) << 1,
                    ..none
                }]);
            }
            Filter::DustAndScratches { radius, threshold } => {
                // Beyond, the CPU takes the median on the layer reduced.
                if f64::from(radius) > MEDIAN_UP_TO {
                    return None;
                }
                return Some(vec![Self {
                    kind: Kind::Median,
                    reach: radius as u32,
                    threshold,
                    ..none
                }]);
            }
            Filter::ClarityTexture {
                texture, clarity, ..
            } => {
                let [fine, broad] = filter.clarity_radii()?;
                let clarity = Self {
                    amount: texture / 100.0,
                    threshold: (f64::from(clarity) / 100.0 * CLARITY_STRENGTH) as f32,
                    ..Self::separable(broad, 5)?
                };
                return Some(vec![Self::separable(fine, KEEP)?, clarity]);
            }
            _ => {}
        }
        let (mode, amount, threshold) = match filter {
            Filter::UnsharpMask {
                amount, threshold, ..
            } => (1, amount, threshold),
            Filter::HighPass { .. } => (2, 0.0, 0.0),
            _ => (0, 0.0, 0.0),
        };
        Some(vec![Self {
            amount,
            threshold,
            ..Self::separable(filter.blur_radius()?, mode)?
        }])
    }
}

/// The exact Gaussian of standard deviation `sigma` along one axis (three sigmas a side),
/// normalized; `None` beyond [`MAX_REACH`].
fn gaussian(sigma: f64) -> Option<Vec<f32>> {
    let reach = (3.0 * sigma).ceil().max(1.0) as i64;
    if reach > i64::from(MAX_REACH) {
        return None;
    }
    let weights: Vec<f64> = (-reach..=reach)
        .map(|i| (-((i * i) as f64) / (2.0 * sigma * sigma)).exp())
        .collect();
    let total: f64 = weights.iter().sum();
    Some(weights.iter().map(|w| (w / total) as f32).collect())
}

/// Contiguous 8-bit RGBA rows (`width` × `height`) as tiles, row-major, edge tiles padded by
/// repeating their last row and column as images pad them. A row of tiles a thread.
fn tiles_of(rows: &[u8], width: u32, height: u32) -> Vec<std::sync::Arc<[u8]>> {
    let t = TILE_SIZE as usize;
    let (width, height) = (width as usize, height as usize);
    let (columns, tile_rows) = (width.div_ceil(t), height.div_ceil(t));
    let mut tiles: Vec<Option<std::sync::Arc<[u8]>>> = vec![None; columns * tile_rows];
    in_parallel(&mut tiles, columns, |tile_row, out| {
        for (col, slot) in out.iter_mut().enumerate() {
            let (x0, y0) = (col * t, tile_row * t);
            let (w, h) = (t.min(width - x0), t.min(height - y0));
            let mut tile = vec![0u8; t * t * 4];
            for ty in 0..t {
                let y = y0 + ty.min(h - 1);
                let line = &rows[(y * width + x0) * 4..(y * width + x0 + w) * 4];
                let out = &mut tile[ty * t * 4..(ty + 1) * t * 4];
                out[..w * 4].copy_from_slice(line);
                let last = [
                    line[(w - 1) * 4],
                    line[(w - 1) * 4 + 1],
                    line[(w - 1) * 4 + 2],
                    line[(w - 1) * 4 + 3],
                ];
                for px in out[w * 4..].chunks_mut(4) {
                    px.copy_from_slice(&last);
                }
            }
            *slot = Some(std::sync::Arc::from(tile));
        }
    });
    tiles.into_iter().flatten().collect()
}

/// The crop's tiles as contiguous 8-bit RGBA rows (`width` × `height`). A row of tiles a thread.
fn rows_of(job: &LookJob, width: u32, height: u32) -> Vec<u8> {
    let t = TILE_SIZE as usize;
    let columns = (width as usize).div_ceil(t);
    let (width, height) = (width as usize, height as usize);
    let mut out = vec![0u8; width * height * 4];
    debug_assert!(height > 0);
    in_parallel(&mut out, t * width * 4, |tile_row, band| {
        for (ty, row) in band.chunks_mut(width * 4).enumerate() {
            for col in 0..columns {
                let Some(tile) = job.tiles.get(tile_row * columns + col) else {
                    continue;
                };
                let x0 = col * t;
                let w = t.min(width - x0);
                row[x0 * 4..(x0 + w) * 4].copy_from_slice(&tile[ty * t * 4..(ty * t + w) * 4]);
            }
        }
    });
    out
}

/// `f(index, chunk)` for each `chunk`-long part of `items` (the last may be shorter), on a few
/// threads: copying a look's pixels takes milliseconds on one.
fn in_parallel<T: Send>(items: &mut [T], chunk: usize, f: impl Fn(usize, &mut [T]) + Sync) {
    let parts = items.len().div_ceil(chunk.max(1));
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(8)
        .min(parts);
    if threads <= 1 {
        for (index, part) in items.chunks_mut(chunk.max(1)).enumerate() {
            f(index, part);
        }
        return;
    }
    let per_thread = parts.div_ceil(threads);
    let f = &f;
    std::thread::scope(|scope| {
        for (n, group) in items.chunks_mut(per_thread * chunk).enumerate() {
            scope.spawn(move || {
                for (i, part) in group.chunks_mut(chunk).enumerate() {
                    f(n * per_thread + i, part);
                }
            });
        }
    });
}
