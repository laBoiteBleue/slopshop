//! Filters on the GPU (ADR 0035): the looks at filtered layers (`stack::LookJob`) the display
//! asks for, computed by compute passes (filter.wgsl) on a thread of their own and read back as
//! the look's image: Gaussian Blur, and Unsharp Mask and High Pass made from it. Only the common
//! case for now: 8-bit RGBA sRGB layers in a perceptual document, steps without a selection, a
//! reach of at most [`MAX_REACH`] pixels; the CPU computes the others (`LookJob::run`).

use std::sync::mpsc;

use slopshop_core::blend::BlendSpace;
use slopshop_core::color::PixelFormat;
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
}

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
        }
    }

    /// The filtered crop of `job`, or `None` when the GPU does not take it (another format, a
    /// selection, too far a reach, a GPU error): the CPU computes it then.
    pub(crate) fn look(&self, job: &LookJob) -> Option<RasterImage> {
        if job.format != PixelFormat::RGBA8_SRGB {
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
        let scopes = [
            wgpu::ErrorFilter::Internal,
            wgpu::ErrorFilter::Validation,
            wgpu::ErrorFilter::OutOfMemory,
        ]
        .map(|filter| self.device.push_error_scope(filter));
        let result = self.blur(job, &kernels, width, height);
        let mut failed = false;
        for scope in scopes.into_iter().rev() {
            failed |= pollster::block_on(scope.pop()).is_some();
        }
        let bytes = result.filter(|_| !failed)?;
        // Level 0 only: a look is shown at the level it is made for.
        let tiles = tiles_of(&bytes, width, height);
        RasterImage::from_level0_tiles_only(job.size, job.format, tiles).ok()
    }

    /// The crop's pixels filtered by each pass in turn, as 8-bit RGBA rows.
    fn blur(&self, job: &LookJob, kernels: &[Pass], width: u32, height: u32) -> Option<Vec<u8>> {
        let input = rows_of(job, width, height);
        let bytes = u64::from(width) * u64::from(height) * 4;
        let device = &self.device;
        let packed = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("filter input"),
            contents: &input,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        // Twice as large when a blur is kept between two passes (in its second half).
        let kept = if kernels.iter().any(|pass| pass.mode == KEEP) {
            2
        } else {
            1
        };
        let rows = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("filter rows"),
            size: bytes * 4 * kept,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("filter output"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("filter readback"),
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let groups = (width.div_ceil(WORKGROUP), height.div_ceil(WORKGROUP));
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("filter"),
        });
        // Whether the output holds a step's result the next one filters.
        let mut fresh = false;
        for pass in kernels {
            if fresh {
                encoder.copy_buffer_to_buffer(&output, 0, &packed, 0, bytes);
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
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
        self.queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            // The receiver only disappears if we already returned; nothing to report then.
            let _ = tx.send(result);
        });
        device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
        rx.recv().ok()?.ok()?;
        let bytes = slice.get_mapped_range().ok()?.to_vec();
        readback.unmap();
        Some(bytes)
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
                let map = step.to_document;
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
/// repeating their last row and column as images pad them.
fn tiles_of(rows: &[u8], width: u32, height: u32) -> Vec<std::sync::Arc<[u8]>> {
    let t = TILE_SIZE as usize;
    let (width, height) = (width as usize, height as usize);
    let (columns, tile_rows) = (width.div_ceil(t), height.div_ceil(t));
    let mut tiles = Vec::with_capacity(columns * tile_rows);
    for tile_row in 0..tile_rows {
        for col in 0..columns {
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
            tiles.push(std::sync::Arc::from(tile));
        }
    }
    tiles
}

/// The crop's tiles as contiguous 8-bit RGBA rows (`width` × `height`).
fn rows_of(job: &LookJob, width: u32, height: u32) -> Vec<u8> {
    let t = TILE_SIZE as usize;
    let columns = (width as usize).div_ceil(t);
    let (width, height) = (width as usize, height as usize);
    let mut out = vec![0u8; width * height * 4];
    for (y, row) in out.chunks_mut(width * 4).enumerate() {
        let (tile_row, ty) = (y / t, y % t);
        for col in 0..columns {
            let Some(tile) = job.tiles.get(tile_row * columns + col) else {
                continue;
            };
            let x0 = col * t;
            let w = t.min(width - x0);
            row[x0 * 4..(x0 + w) * 4].copy_from_slice(&tile[ty * t * 4..(ty * t + w) * 4]);
        }
        debug_assert!(height > 0);
    }
    out
}
