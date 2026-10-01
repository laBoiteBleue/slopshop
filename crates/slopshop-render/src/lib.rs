//! SlopShop GPU rendering.
//!
//! Renders a view of a document (a region at a given scale) into a display frame. Only the
//! output-sized area is ever computed or read back, never the whole document. Works headless:
//! no window or surface is required; a window surface can be presented to directly
//! ([`present`]).
//!
//! Export reads full-precision working-space pixels of any document region instead
//! ([`Renderer::render_region`]).
//!
//! All methods block the calling thread (GPU submission and readback): call them from worker
//! threads, never from a UI thread.

pub mod present;
mod region;
mod tiles;

pub use region::{export_renderer, export_source};

use std::collections::HashSet;
use std::fmt;
use std::sync::{Mutex, mpsc};

use slopshop_core::adjust::{Adjustment, SRGB_LUMA};
use slopshop_core::color::{
    AlphaMode, ColorSpace, Mat3, PixelFormat, TransferFunction, WORKING_SPACE,
};
use slopshop_core::composite::{Step, steps};
use slopshop_core::document::MAX_GROUP_DEPTH;
use slopshop_core::raster::TILE_SIZE;
use slopshop_core::resample::{self, Filter, Resampling};
use slopshop_core::tile::TileCoord;
use slopshop_core::view::ViewTransform;
use slopshop_core::{
    Affine, BlendMode, BlendSpace, Document, Layer, LayerContent, RasterImage, Rect, Size,
};

use crate::tiles::{GpuTileFormat, TileCache, TileKey, gpu_texels};

/// A rendered frame, tightly packed rows, top to bottom.
#[derive(Debug, Clone)]
pub struct Frame {
    pub size: Size,
    pub format: PixelFormat,
    pub data: Vec<u8>,
}

#[derive(Debug)]
pub enum RenderError {
    NoAdapter(String),
    Device(String),
    EmptyOutput,
    OutputTooLarge {
        size: Size,
        max_bytes: u64,
    },
    Readback(String),
    /// A window surface could not be created, configured or acquired.
    Surface(String),
    /// A region to render does not lie within the document.
    RegionOutsideDocument {
        region: Rect,
        document: Size,
    },
    /// The buffer given for a region does not hold exactly its RGBA f32 samples.
    RegionBufferLength {
        expected: u64,
        actual: usize,
    },
    /// More distinct raster images of one sample type are visible in a region than the tile
    /// cache holds: not even one tile of the region can be rendered at full resolution.
    TooManyLayers {
        images: usize,
        capacity: u32,
    },
    /// The GPU ran out of memory (e.g. for a buffer or a tile array).
    OutOfMemory(String),
    /// Any other error reported by the GPU device (validation, internal).
    Gpu(String),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenderError::NoAdapter(e) => write!(f, "no suitable GPU adapter: {e}"),
            RenderError::Device(e) => write!(f, "could not create GPU device: {e}"),
            RenderError::EmptyOutput => write!(f, "output size is empty"),
            RenderError::OutputTooLarge { size, max_bytes } => write!(
                f,
                "output {}x{} exceeds the GPU buffer limit of {max_bytes} bytes",
                size.width, size.height
            ),
            RenderError::Readback(e) => write!(f, "GPU readback failed: {e}"),
            RenderError::Surface(e) => write!(f, "window surface: {e}"),
            RenderError::RegionOutsideDocument { region, document } => write!(
                f,
                "region {}x{} at ({}, {}) is outside the {}x{} document",
                region.width, region.height, region.x, region.y, document.width, document.height
            ),
            RenderError::RegionBufferLength { expected, actual } => write!(
                f,
                "region buffer holds {actual} samples, {expected} expected"
            ),
            RenderError::TooManyLayers { images, capacity } => write!(
                f,
                "{images} raster images of one sample type exceed the tile cache ({capacity} tiles)"
            ),
            RenderError::OutOfMemory(e) => write!(f, "GPU out of memory: {e}"),
            RenderError::Gpu(e) => write!(f, "GPU error: {e}"),
        }
    }
}

impl std::error::Error for RenderError {}

/// Human-readable description of the GPU in use.
#[derive(Debug, Clone)]
pub struct AdapterSummary {
    pub name: String,
    pub backend: String,
    pub device_type: String,
    pub driver: String,
}

/// GPU device plus the viewport compositing pipeline and the raster tile cache.
#[derive(Debug)]
pub struct Renderer {
    /// Kept to create window surfaces later ([`present`]): surfaces must come from the
    /// instance that owns the adapter.
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    adapter_info: wgpu::AdapterInfo,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    /// Export: `export_main` in composite.wgsl ([`Renderer::render_region`]).
    export_pipeline: wgpu::ComputePipeline,
    export_bind_group_layout: wgpu::BindGroupLayout,
    /// Largest storage buffer that can be bound (and copied from) in one dispatch.
    max_output_bytes: u64,
    /// Largest dispatch along one axis, in pixels.
    max_dispatch_pixels: u32,
    /// One cache per storage class ([`GpuTileFormat`]), created on first use: documents
    /// without raster layers of a class need no tile memory for it.
    tile_caches: Mutex<[Option<TileCache>; 4]>,
    tile_capacity: [u32; 4],
    placeholder_tiles: [wgpu::TextureView; 4],
    /// The resampling kernel table (ADR 0018, `resample::weight_table`), as a uniform block.
    ewa_table: wgpu::Buffer,
}

/// Upper bound on cached tiles per storage class.
const MAX_TILE_CAPACITY: u32 = 1024;
/// VRAM budget per storage class: 1024 8-bit tiles, 768 16-bit tiles or 384 f32 tiles, enough
/// for a 4K viewport at full detail in the common cases (coarser levels are used otherwise).
const TILE_BUDGET_BYTES: u64 = 384 * 1024 * 1024;
/// Marks a tile that is not resident (see `NO_TILE` in composite.wgsl).
const NO_TILE: u32 = u32::MAX;
const KIND_FILL: u32 = 0;
const KIND_RASTER: u32 = 1;
/// Group steps (ADR 0015, `composite::Step`): push the accumulator, then pop and combine.
const KIND_GROUP_BEGIN: u32 = 2;
const KIND_GROUP_END: u32 = 3;
/// An adjustment layer (ADR 0020): `format` holds the adjustment's index, `color` and
/// `transfer.x` its parameters.
const KIND_ADJUST: u32 = 4;
/// Layer flags (see composite.wgsl).
const FLAG_PREMULTIPLIED: u32 = 1;
/// Gray source: green and blue of its GPU texels are copies of red ([`gpu_texels`]).
const FLAG_GRAY: u32 = 2;
/// The document blends in perceptual space (ADR 0012).
const FLAG_PERCEPTUAL: u32 = 4;
/// The layer has an enabled mask, described by the `mask_*` fields (ADR 0014).
const FLAG_MASK: u32 = 8;
/// The layer's own alpha is ignored (a mask made from its transparency, ADR 0014).
const FLAG_IGNORE_ALPHA: u32 = 16;
/// A group step of an isolated group (else it passes through).
const FLAG_ISOLATED: u32 = 32;
/// Blended atop the accumulator, keeping its coverage: a clipped layer (ADR 0016).
const FLAG_ATOP: u32 = 64;
/// The layer's blend mode ([`BlendMode::index`]) is stored in the flags from this bit.
const BLEND_SHIFT: u32 = 8;
/// Filters of resampled rasters (`resample_q.w` in composite.wgsl; 0: a whole-pixel offset).
const RESAMPLE_NEAREST: u32 = 1;
const RESAMPLE_EWA: u32 = 2;

const WORKGROUP_SIZE: u32 = 8;
const OUTPUT_FORMAT: PixelFormat = PixelFormat::RGBA8_SRGB;

impl Renderer {
    /// Pick a GPU and build the pipelines. Blocking, and can take a noticeable time.
    pub fn new() -> Result<Self, RenderError> {
        // Windows: DX12, whose swapchains (flip model) are what direct presentation needs and
        // which reconfigures much faster than Vulkan on resize. `WGPU_BACKEND` overrides.
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        if cfg!(windows) {
            descriptor.backends = wgpu::Backends::DX12;
        }
        let instance = wgpu::Instance::new(descriptor.with_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            // `SLOPSHOP_SOFTWARE_GPU=1`: the software adapter (WARP on Windows), as in CI.
            force_fallback_adapter: std::env::var("SLOPSHOP_SOFTWARE_GPU").as_deref() == Ok("1"),
            compatible_surface: None,
            // Anti-fingerprinting for untrusted content; irrelevant for a native app.
            apply_limit_buckets: false,
        }))
        .map_err(|e| RenderError::NoAdapter(e.to_string()))?;

        if !adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::COMPUTE_SHADERS)
        {
            return Err(RenderError::NoAdapter(
                "adapter does not support compute shaders".into(),
            ));
        }

        // Baseline limits, raised to what the adapter offers for buffer sizes so that large
        // (e.g. 8K display) viewports fit in one storage buffer, and for texture sizes so that
        // window surfaces larger than the baseline's 2048 px can be configured.
        let adapter_limits = adapter.limits();
        let required_limits = wgpu::Limits {
            max_texture_dimension_2d: adapter_limits.max_texture_dimension_2d,
            max_storage_buffer_binding_size: adapter_limits.max_storage_buffer_binding_size,
            max_buffer_size: adapter_limits.max_buffer_size,
            max_texture_array_layers: adapter_limits.max_texture_array_layers,
            ..wgpu::Limits::downlevel_defaults()
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("slopshop"),
            required_features: wgpu::Features::empty(),
            required_limits: required_limits.clone(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
        }))
        .map_err(|e| RenderError::Device(e.to_string()))?;

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("composite.wgsl"),
            source: wgpu::ShaderSource::Wgsl(shader_source().into()),
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
        let uniform = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        // Layers, tile table, tile arrays and the resampling kernel, shared by both entry points.
        // The kernel is a uniform block: downlevel devices bind at most 4 storage buffers per
        // stage, and export already uses 4.
        let shared = [
            storage(1, true),
            storage(3, true),
            tile_binding(4, GpuTileFormat::Unorm8),
            tile_binding(5, GpuTileFormat::Uint16),
            tile_binding(6, GpuTileFormat::Float16),
            tile_binding(7, GpuTileFormat::Float32),
            uniform(11),
        ];
        let (bind_group_layout, pipeline) = compute_pipeline(
            &device,
            &module,
            "main",
            &[&shared[..], &[uniform(0), storage(2, false)]].concat(),
        );
        let (export_bind_group_layout, export_pipeline) = compute_pipeline(
            &device,
            &module,
            "export_main",
            &[
                &shared[..],
                &[uniform(8), storage(9, false), storage(10, false)],
            ]
            .concat(),
        );

        let max_output_bytes = required_limits
            .max_storage_buffer_binding_size
            .min(required_limits.max_buffer_size);
        let max_dispatch_pixels = required_limits
            .max_compute_workgroups_per_dimension
            .saturating_mul(WORKGROUP_SIZE);
        let placeholder_tiles = GpuTileFormat::ALL.map(|f| tiles::placeholder_view(&device, f));
        let ewa_table = {
            use wgpu::util::DeviceExt;
            let bytes: Vec<u8> = resample::weight_table()
                .iter()
                .flat_map(|w| w.to_le_bytes())
                .collect();
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("resampling kernel"),
                contents: &bytes,
                usage: wgpu::BufferUsages::UNIFORM,
            })
        };
        let tile_capacity = GpuTileFormat::ALL.map(|f| {
            let budget = u32::try_from(TILE_BUDGET_BYTES / f.tile_bytes()).unwrap_or(u32::MAX);
            budget
                .min(MAX_TILE_CAPACITY)
                .min(required_limits.max_texture_array_layers)
                .max(1)
        });
        Ok(Self {
            instance,
            adapter_info: adapter.get_info(),
            adapter,
            device,
            queue,
            pipeline,
            bind_group_layout,
            export_pipeline,
            export_bind_group_layout,
            max_output_bytes,
            max_dispatch_pixels,
            tile_caches: Mutex::new([None, None, None, None]),
            tile_capacity,
            placeholder_tiles,
            ewa_table,
        })
    }

    pub fn adapter_summary(&self) -> AdapterSummary {
        let info = &self.adapter_info;
        AdapterSummary {
            name: info.name.clone(),
            backend: info.backend.to_string(),
            device_type: format!("{:?}", info.device_type),
            driver: format!("{} {}", info.driver, info.driver_info)
                .trim()
                .to_owned(),
        }
    }

    /// Render `document` as seen through `view` into an `output`-sized RGBA8 sRGB frame.
    pub fn render_view(
        &self,
        document: &Document,
        view: ViewTransform,
        output: Size,
    ) -> Result<Frame, RenderError> {
        let mut data = Vec::new();
        self.render_view_into(document, view, output, &mut data)?;
        Ok(Frame {
            size: output,
            format: OUTPUT_FORMAT,
            data,
        })
    }

    /// Size in bytes of an `output`-sized frame, or why it cannot be rendered (empty, or larger
    /// than the GPU buffer limit). Callers can check before allocating anything.
    pub fn output_byte_len(&self, output: Size) -> Result<u64, RenderError> {
        if output.is_empty() {
            return Err(RenderError::EmptyOutput);
        }
        output
            .pixel_count()
            .checked_mul(u64::from(OUTPUT_FORMAT.bytes_per_pixel()))
            .filter(|&len| len <= self.max_output_bytes)
            .ok_or(RenderError::OutputTooLarge {
                size: output,
                max_bytes: self.max_output_bytes,
            })
    }

    /// Like [`Self::render_view`], but appends the RGBA8 sRGB pixels to `out`. Lets callers put
    /// a header before the pixels (or reuse an allocation) without copying the frame again.
    /// On error, `out` is left as it was.
    pub fn render_view_into(
        &self,
        document: &Document,
        view: ViewTransform,
        output: Size,
        out: &mut Vec<u8>,
    ) -> Result<(), RenderError> {
        let byte_len = self.output_byte_len(output)?;
        let len = out.len();
        self.capture_errors(|| {
            let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("composite readback"),
                size: byte_len,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            self.composite(document, view, output, |encoder, pixels| {
                encoder.copy_buffer_to_buffer(pixels, 0, &readback, 0, byte_len);
            })?;
            self.read_buffer_into(&readback, out)
        })
        // A GPU error found after the pixels were appended: leave `out` as it was.
        .inspect_err(|_| out.truncate(len))
    }

    /// Run `f`, capturing the GPU errors it causes on this thread (out of memory, validation,
    /// internal) as a [`RenderError`]. Without this, wgpu's default handler panics, possibly
    /// while a lock is held. A captured error takes precedence over the result of `f`, which
    /// may have gone on with invalid GPU objects.
    fn capture_errors<T>(
        &self,
        f: impl FnOnce() -> Result<T, RenderError>,
    ) -> Result<T, RenderError> {
        let scopes = [
            wgpu::ErrorFilter::Internal,
            wgpu::ErrorFilter::Validation,
            wgpu::ErrorFilter::OutOfMemory,
        ]
        .map(|filter| self.device.push_error_scope(filter));
        let result = f();
        // Scopes pop in reverse order; the first error (out of memory first) is the cause.
        let mut captured = None;
        for scope in scopes.into_iter().rev() {
            if let Some(error) = pollster::block_on(scope.pop()) {
                captured.get_or_insert(error);
            }
        }
        match captured {
            Some(wgpu::Error::OutOfMemory { source }) => {
                Err(RenderError::OutOfMemory(source.to_string()))
            }
            Some(error) => Err(RenderError::Gpu(error.to_string())),
            None => result,
        }
    }

    /// Composite `view` of `document` into an `output`-sized buffer of packed RGBA8 sRGB pixels
    /// (rows of `output.width` pixels), let `finish` record what to do with it (read back, copy
    /// to a surface…), and submit. Does not wait for the GPU.
    fn composite(
        &self,
        document: &Document,
        view: ViewTransform,
        output: Size,
        finish: impl FnOnce(&mut wgpu::CommandEncoder, &wgpu::Buffer),
    ) -> Result<(), RenderError> {
        let byte_len = self.output_byte_len(output)?;

        // The cache lock is held until the GPU work is submitted: tiles resident for this frame
        // must not be evicted before.
        let mut cache_guard = self.tile_caches.lock().unwrap_or_else(|poisoned| {
            // A panic while the caches were locked may have left them inconsistent: start
            // afresh rather than failing every later frame.
            let mut guard = poisoned.into_inner();
            *guard = [None, None, None, None];
            self.tile_caches.clear_poison();
            guard
        });
        let result = self.capture_errors(|| {
            self.composite_locked(document, view, output, byte_len, &mut cache_guard, finish)
        });
        if result.is_err() {
            // Tiles may have been recorded as resident in a cache whose texture or upload
            // failed: rebuild the caches on the next frame.
            *cache_guard = [None, None, None, None];
        }
        result
    }

    /// [`Self::composite`] with the tile caches locked.
    fn composite_locked(
        &self,
        document: &Document,
        view: ViewTransform,
        output: Size,
        byte_len: u64,
        cache_guard: &mut [Option<TileCache>; 4],
        finish: impl FnOnce(&mut wgpu::CommandEncoder, &wgpu::Buffer),
    ) -> Result<(), RenderError> {
        let layers = self.prepare_layers(document, view, output, cache_guard);
        let params = params_bytes(document.size(), view, output, layers.count);

        use wgpu::util::DeviceExt;
        let params_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("composite params"),
                contents: &params,
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let layer_buffers = self.layer_buffers(layers);
        // Allocated per frame for now; pooling can come once profiling says it matters.
        let output_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("composite output"),
            size: byte_len,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let bind_group = self.bind_group(
            &self.bind_group_layout,
            &layer_buffers,
            self.tile_views(cache_guard),
            [
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: output_buffer.as_entire_binding(),
                },
            ],
        );

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("composite"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("composite"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(
                output.width.div_ceil(WORKGROUP_SIZE),
                output.height.div_ceil(WORKGROUP_SIZE),
                1,
            );
        }
        finish(&mut encoder, &output_buffer);
        self.queue.submit([encoder.finish()]);
        Ok(())
    }

    /// Map a `MAP_READ` buffer and append its content to `out`. This copy is required: mapped
    /// GPU memory cannot outlive the mapping.
    fn read_buffer_into(
        &self,
        buffer: &wgpu::Buffer,
        out: &mut Vec<u8>,
    ) -> Result<(), RenderError> {
        let slice = buffer.slice(..);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            // The receiver only disappears if we already returned; nothing to report then.
            let _ = tx.send(result);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| RenderError::Readback(e.to_string()))?;
        rx.recv()
            .map_err(|e| RenderError::Readback(e.to_string()))?
            .map_err(|e| RenderError::Readback(e.to_string()))?;
        {
            let mapped = slice
                .get_mapped_range()
                .map_err(|e| RenderError::Readback(e.to_string()))?;
            out.extend_from_slice(&mapped);
        }
        buffer.unmap();
        Ok(())
    }
}

/// Size of one `Layer` in composite.wgsl.
const LAYER_BYTES: usize = 304;

/// GPU-ready description of the visible layers of one frame.
struct PreparedLayers {
    count: u32,
    /// `count` × [`LAYER_BYTES`].
    bytes: Vec<u8>,
    /// Cache slots of the visible raster tiles, per layer range.
    tile_table: Vec<u32>,
}

impl Renderer {
    /// Encode the visible layers bottom to top, making the raster tiles they need resident.
    fn prepare_layers(
        &self,
        document: &Document,
        view: ViewTransform,
        output: Size,
        caches: &mut [Option<TileCache>; 4],
    ) -> PreparedLayers {
        let visible_doc = visible_document_rect(document.size(), view, output);
        let steps = steps(document);

        // Plan the pyramid level of every raster layer together, so that all visible tiles
        // fit in the cache: coarser levels rather than missing layers.
        // Two plans per step: its raster, then its enabled mask (ADR 0014), planned together.
        let mut plans: Vec<Option<RasterPlan<'_>>> = steps
            .iter()
            .flat_map(|step| {
                step_rasters(step).map(|raster| {
                    let (image, transform) = raster?;
                    RasterPlan::new(image, visible_doc?, transform, view.scale)
                })
            })
            .collect();
        let mut tables: Vec<Vec<u32>> = vec![Vec::new(); plans.len()];
        if plans.iter().any(Option::is_some) {
            fit_tile_budget(&mut plans, self.tile_capacity);
            for format in GpuTileFormat::ALL {
                if plans.iter().flatten().any(|p| p.format == format) {
                    let capacity = self.tile_capacity[format.index()];
                    caches[format.index()]
                        .get_or_insert_with(|| TileCache::new(&self.device, format, capacity))
                        .begin_frame();
                }
            }
            // Top layer first: if even the coarsest levels do not fit (more visible raster
            // layers than cache slots), the layers left out are the bottom ones.
            for (plan, table) in plans.iter().zip(tables.iter_mut()).rev() {
                if let Some(plan) = plan
                    && let Some(cache) = caches[plan.format.index()].as_mut()
                {
                    *table = self.upload(plan, cache);
                }
            }
        }
        encode_layers(&steps, &plans, tables, document.blend_space())
    }

    /// Layer and tile table buffers of prepared layers.
    fn layer_buffers(&self, prepared: PreparedLayers) -> LayerBuffers {
        use wgpu::util::DeviceExt;
        // A binding cannot be empty: always upload at least one (unused) entry.
        let mut layer_bytes = prepared.bytes;
        if layer_bytes.is_empty() {
            layer_bytes.resize(LAYER_BYTES, 0);
        }
        let layers = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("composite layers"),
                contents: &layer_bytes,
                usage: wgpu::BufferUsages::STORAGE,
            });
        let mut table = prepared.tile_table;
        if table.is_empty() {
            table.push(NO_TILE);
        }
        let table_bytes: Vec<u8> = table.iter().flat_map(|v| v.to_le_bytes()).collect();
        let table = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("composite tile table"),
                contents: &table_bytes,
                usage: wgpu::BufferUsages::STORAGE,
            });
        LayerBuffers { layers, table }
    }

    /// The tile array of each storage class, or a placeholder where there is no cache.
    fn tile_views<'a>(&'a self, caches: &'a [Option<TileCache>; 4]) -> [&'a wgpu::TextureView; 4] {
        std::array::from_fn(|i| {
            caches[i]
                .as_ref()
                .map_or(&self.placeholder_tiles[i], TileCache::view)
        })
    }

    /// A bind group of the bindings shared by both entry points plus an entry point's own
    /// (params and output; export adds its non-finite counter).
    fn bind_group<const N: usize>(
        &self,
        layout: &wgpu::BindGroupLayout,
        buffers: &LayerBuffers,
        tile_views: [&wgpu::TextureView; 4],
        own: [wgpu::BindGroupEntry<'_>; N],
    ) -> wgpu::BindGroup {
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 1,
                resource: buffers.layers.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: buffers.table.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 11,
                resource: self.ewa_table.as_entire_binding(),
            },
        ];
        entries.extend(
            (4..)
                .zip(tile_views)
                .map(|(binding, view)| wgpu::BindGroupEntry {
                    binding,
                    resource: wgpu::BindingResource::TextureView(view),
                }),
        );
        entries.extend(own);
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("composite"),
            layout,
            entries: &entries,
        })
    }
}

/// The rasters a step samples, with their transforms to the document: a raster layer's image,
/// then its (or a group's) enabled mask.
fn step_rasters<'a>(step: &Step<'a>) -> [Option<(&'a RasterImage, Affine)>; 2] {
    match step {
        Step::Layer {
            layer, transform, ..
        } => {
            let content = match &layer.content {
                LayerContent::Raster { image } => Some((image.as_ref(), *transform)),
                _ => None,
            };
            [
                content,
                enabled_mask(layer).map(|image| (image, *transform)),
            ]
        }
        Step::Adjust {
            layer, transform, ..
        } => [None, enabled_mask(layer).map(|image| (image, *transform))],
        Step::Begin { .. } => [None, None],
        Step::End {
            mask,
            mask_transform,
            ..
        } => [None, mask.map(|m| (m.image.as_ref(), *mask_transform))],
    }
}

/// A document rectangle `[x0, y0, x1, y1]` seen from an image at `offset`.
fn shifted(area: [f64; 4], offset: [i32; 2]) -> [f64; 4] {
    let (x, y) = (f64::from(offset[0]), f64::from(offset[1]));
    [area[0] - x, area[1] - y, area[2] - x, area[3] - y]
}

/// The image of a layer's mask, when it has an enabled one.
fn enabled_mask(layer: &Layer) -> Option<&RasterImage> {
    layer
        .mask
        .as_ref()
        .filter(|mask| mask.enabled)
        .map(|mask| mask.image.as_ref())
}

/// Encode `steps` (bottom to top) with their plans and tile slots: two per step, its raster
/// then its enabled mask. Raster layers without a plan (nothing of them to sample) are left out,
/// and so are layers whose enabled mask has none (the mask hides them there); a group whose mask
/// has none keeps its steps, at opacity 0 (what is below stays).
fn encode_layers(
    steps: &[Step<'_>],
    plans: &[Option<RasterPlan<'_>>],
    tables: Vec<Vec<u32>>,
    blend_space: BlendSpace,
) -> PreparedLayers {
    let mut prepared = PreparedLayers {
        count: 0,
        bytes: Vec::new(),
        tile_table: Vec::new(),
    };
    let perceptual = if blend_space == BlendSpace::Perceptual {
        FLAG_PERCEPTUAL
    } else {
        0
    };
    let mut tables = tables.into_iter();
    for (i, step) in steps.iter().enumerate() {
        let (plan, mask_plan) = (&plans[2 * i], &plans[2 * i + 1]);
        let table = tables.next().unwrap_or_default();
        let mask_table = tables.next().unwrap_or_default();
        let (layer, mode, opacity, atop) = match step {
            Step::Layer {
                layer,
                mode,
                opacity,
                atop,
                ..
            } => (*layer, *mode, *opacity, *atop),
            Step::Adjust {
                layer,
                adjustment,
                opacity,
                ..
            } => {
                // Hidden where its enabled mask has nothing to sample.
                if enabled_mask(layer).is_some() && mask_plan.is_none() {
                    continue;
                }
                // The 16 parameters in fields an adjustment has no other use for.
                let mut p = adjustment.params();
                if let Adjustment::PhotoFilter { color, .. } = adjustment {
                    // The shader multiplies by the linear sRGB color.
                    let filter = slopshop_core::adjust::filter_color(*color);
                    p[..3].copy_from_slice(&filter.map(|v| v as f32));
                }
                let vec4 = |at: usize| [p[at], p[at + 1], p[at + 2], p[at + 3]];
                let mut fields = LayerFields {
                    kind: KIND_ADJUST,
                    flags: perceptual,
                    opacity: *opacity,
                    format: adjustment.index(),
                    color: vec4(0),
                    transfer: vec4(4),
                    transfer2: vec4(8),
                    matrix: [vec4(12), [0.0; 4], [0.0; 4]],
                    ..LayerFields::default()
                };
                // Curves' lookup tables (composite, red, green, blue), after the tile slots.
                if let Some(curves) = adjustment.curves() {
                    fields.table_offset = prepared.tile_table.len() as u32;
                    for curve in &curves {
                        prepared
                            .tile_table
                            .extend(curve.lut().iter().map(|v| v.to_bits()));
                    }
                }
                if let Some(mask) = mask_plan {
                    set_mask_fields(&mut fields, mask, &mut prepared.tile_table, mask_table);
                }
                fields.write(&mut prepared.bytes);
                prepared.count += 1;
                continue;
            }
            Step::Begin { isolated } => {
                let fields = LayerFields {
                    kind: KIND_GROUP_BEGIN,
                    flags: perceptual | if *isolated { FLAG_ISOLATED } else { 0 },
                    ..LayerFields::default()
                };
                fields.write(&mut prepared.bytes);
                prepared.count += 1;
                continue;
            }
            Step::End {
                mask,
                mode,
                opacity,
                isolated,
                atop,
                ..
            } => {
                let hidden = mask.is_some() && mask_plan.is_none();
                let mut fields = LayerFields {
                    kind: KIND_GROUP_END,
                    flags: mode.index() << BLEND_SHIFT
                        | perceptual
                        | if *isolated { FLAG_ISOLATED } else { 0 }
                        | if *atop { FLAG_ATOP } else { 0 },
                    opacity: if hidden { 0.0 } else { *opacity },
                    ..LayerFields::default()
                };
                if let Some(mask) = mask_plan {
                    set_mask_fields(&mut fields, mask, &mut prepared.tile_table, mask_table);
                }
                fields.write(&mut prepared.bytes);
                prepared.count += 1;
                continue;
            }
        };
        if enabled_mask(layer).is_some() && mask_plan.is_none() {
            continue;
        }
        let mut fields = LayerFields {
            flags: mode.index() << BLEND_SHIFT | perceptual | if atop { FLAG_ATOP } else { 0 },
            ..LayerFields::default()
        };
        let replaces_alpha = layer.mask.as_ref().is_some_and(|m| m.replaces_alpha);
        match &layer.content {
            LayerContent::Fill { color } => {
                fields.kind = KIND_FILL;
                let alpha = if replaces_alpha { 1.0 } else { color.a };
                let a = alpha * opacity;
                fields.color = [color.r * a, color.g * a, color.b * a, a];
            }
            LayerContent::Raster { .. } => {
                // Not visible in this view or region: nothing to sample.
                let Some(plan) = plan else { continue };
                fields.kind = KIND_RASTER;
                fields.opacity = opacity;
                fields.offset = plan.offset();
                fields.resample = resample_fields(plan);
                let range = plan.range();
                let size = plan.image.levels()[plan.level].size();
                fields.level_scale = plan.factor() as f32;
                fields.table_offset = prepared.tile_table.len() as u32;
                fields.tile_origin = [range.x, range.y];
                fields.tile_count = [range.width, range.height];
                fields.level_size = [size.width, size.height];
                fields.format = plan.format.index() as u32;
                let stored = plan.image.stored_format();
                if stored.alpha == AlphaMode::Premultiplied {
                    fields.flags |= FLAG_PREMULTIPLIED;
                }
                if stored.layout.is_gray() {
                    fields.flags |= FLAG_GRAY;
                }
                (fields.transfer, fields.transfer2) = transfer_fields(stored.color_space.transfer);
                fields.matrix = matrix_rows(&plan.image.matrix_to(&WORKING_SPACE));
                if replaces_alpha {
                    fields.flags |= FLAG_IGNORE_ALPHA;
                }
                prepared.tile_table.extend(table);
            }
            // Groups and adjustments are steps of their own.
            LayerContent::Group { .. } | LayerContent::Adjustment { .. } => continue,
        }
        if let Some(mask) = mask_plan {
            set_mask_fields(&mut fields, mask, &mut prepared.tile_table, mask_table);
        }
        fields.write(&mut prepared.bytes);
        prepared.count += 1;
    }
    prepared
}

/// Describe an enabled mask's plan in `fields`, its tile slots appended to `tile_table`.
fn set_mask_fields(
    fields: &mut LayerFields,
    mask: &RasterPlan<'_>,
    tile_table: &mut Vec<u32>,
    slots: Vec<u32>,
) {
    fields.flags |= FLAG_MASK;
    let range = mask.range();
    let size = mask.image.levels()[mask.level].size();
    fields.mask_level_scale = mask.factor() as f32;
    fields.mask_table_offset = tile_table.len() as u32;
    fields.mask_tile_origin = [range.x, range.y];
    fields.mask_tile_count = [range.width, range.height];
    fields.mask_level_size = [size.width, size.height];
    fields.mask_format = mask.format.index() as u32;
    fields.mask_offset = mask.offset();
    fields.mask_resample = resample_fields(mask);
    tile_table.extend(slots);
}

/// The `resample_*` fields of composite.wgsl for a plan: zeros for a whole-pixel offset; else
/// the map from document points to the planned level's texels (and the texel box's half-size)
/// in the first two rows, the ellipse's quadratic form and the filter in the third.
fn resample_fields(plan: &RasterPlan<'_>) -> [[f32; 4]; 3] {
    let Some(r) = plan.resampling() else {
        return [[0.0; 4]; 3];
    };
    let t = r.to_texel;
    let (filter, q, extent) = match r.filter {
        Filter::Nearest => (RESAMPLE_NEAREST, [0.0; 3], [0.5, 0.5]),
        Filter::Ewa { q, extent } => (RESAMPLE_EWA, q, extent),
    };
    [
        [t.a as f32, t.c as f32, t.e as f32, extent[0] as f32],
        [t.b as f32, t.d as f32, t.f as f32, extent[1] as f32],
        [q[0] as f32, q[1] as f32, q[2] as f32, filter as f32],
    ]
}

/// Layer and tile table buffers bound for one dispatch.
struct LayerBuffers {
    layers: wgpu::Buffer,
    table: wgpu::Buffer,
}

impl Renderer {
    /// Make the planned tiles resident; returns their cache slots (row-major).
    fn upload(&self, plan: &RasterPlan<'_>, cache: &mut TileCache) -> Vec<u32> {
        let level = &plan.image.levels()[plan.level];
        let stored = plan.image.stored_format();
        plan.keys()
            .map(|key| {
                let coord = TileCoord {
                    col: key.col,
                    row: key.row,
                };
                level
                    .tile(coord)
                    .and_then(|tile| cache.ensure(&self.queue, key, || gpu_texels(tile, stored)))
                    .unwrap_or(NO_TILE)
            })
            .collect()
    }

    /// Limit every tile cache to `capacity` tiles (at least 1). For tests and
    /// memory-constrained setups; resets the caches.
    #[doc(hidden)]
    pub fn with_tile_capacity(mut self, capacity: u32) -> Self {
        self.tile_capacity = self.tile_capacity.map(|c| capacity.clamp(1, c.max(1)));
        self.tile_caches = Mutex::new([None, None, None, None]);
        self
    }
}

/// How one visible raster layer is sampled in a frame.
struct RasterPlan<'a> {
    image: &'a RasterImage,
    format: GpuTileFormat,
    /// Document area to cover: `[x0, y0, x1, y1]`.
    area: [f64; 4],
    place: Place,
    level: usize,
}

/// Where a planned raster is in the document.
#[derive(Debug, Clone, Copy)]
enum Place {
    /// A whole-pixel offset (ADR 0017), clamped to the shader's `i32`.
    Offset([i32; 2]),
    /// Any other transform: resampled (ADR 0018), from the plan's level.
    Resampled(Resampling),
}

impl<'a> RasterPlan<'a> {
    /// Sampling `image`, placed by `transform`, over the document `area` for output pixels of
    /// `scale` document pixels: at the finest level whose pixels are not smaller than output
    /// pixels (for a whole-pixel offset), or the resampling's level. `None` for a transform
    /// that is not invertible (edits refuse them).
    fn new(image: &'a RasterImage, area: [f64; 4], transform: Affine, scale: f64) -> Option<Self> {
        let coarsest = image.levels().len() - 1;
        let (place, level) = match transform.integer_translation() {
            Some((x, y)) => {
                let clamp = |v: i64| v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
                let level = if scale > 1.0 {
                    (scale.log2().floor() as usize).min(coarsest)
                } else {
                    0
                };
                (Place::Offset([clamp(x), clamp(y)]), level)
            }
            None => {
                let r = Resampling::new(transform, scale, image.levels().len())?;
                (Place::Resampled(r), r.level)
            }
        };
        Some(Self {
            image,
            format: GpuTileFormat::for_sample(image.stored_format().sample),
            area,
            place,
            level,
        })
    }

    /// Level 0 (or the resampling's level), whatever the tile budget (export).
    fn full_resolution(image: &'a RasterImage, area: [f64; 4], transform: Affine) -> Option<Self> {
        Self::new(image, area, transform, 1.0)
    }

    /// The whole-pixel offset; zero when resampled.
    fn offset(&self) -> [i32; 2] {
        match self.place {
            Place::Offset(offset) => offset,
            Place::Resampled(_) => [0, 0],
        }
    }

    /// The resampling from the planned level, when resampled.
    fn resampling(&self) -> Option<Resampling> {
        match self.place {
            Place::Offset(_) => None,
            Place::Resampled(r) => Some(r.at_level(self.level)),
        }
    }

    /// Document pixels per pixel of the planned level.
    fn factor(&self) -> f64 {
        f64::from(1u32 << self.level)
    }

    fn can_coarsen(&self) -> bool {
        self.level + 1 < self.image.levels().len()
    }

    /// Visible tiles of the planned level.
    fn range(&self) -> Rect {
        let grid = self.image.levels()[self.level].grid();
        // The image's area (level-0 pixels) that the document area reads.
        let visible = match self.resampling() {
            Some(r) => r.source_area(self.area),
            None => shifted(self.area, self.offset()),
        };
        tile_range(visible, self.factor(), grid.columns(), grid.rows())
    }

    fn keys(&self) -> impl Iterator<Item = TileKey> + '_ {
        let range = self.range();
        let (image, level) = (self.image.id(), self.level as u32);
        (range.y..range.y + range.height).flat_map(move |row| {
            (range.x..range.x + range.width).map(move |col| TileKey {
                image,
                level,
                col,
                row,
            })
        })
    }
}

/// Coarsen plans until the distinct visible tiles of each storage class fit in that class's
/// cache: each step coarsens the plan of an over-budget class that needs the most tiles. Layers
/// sharing an image at the same level count once.
fn fit_tile_budget(plans: &mut [Option<RasterPlan<'_>>], capacity: [u32; 4]) {
    for format in GpuTileFormat::ALL {
        loop {
            let needed: HashSet<TileKey> = plans
                .iter()
                .flatten()
                .filter(|p| p.format == format)
                .flat_map(RasterPlan::keys)
                .collect();
            if needed.len() <= capacity[format.index()] as usize {
                break;
            }
            let largest = plans
                .iter_mut()
                .flatten()
                .filter(|plan| plan.format == format && plan.can_coarsen())
                .max_by_key(|plan| plan.range().width * plan.range().height);
            match largest {
                Some(plan) => plan.level += 1,
                // All at their coarsest level: the upload order decides what is left out.
                None => break,
            }
        }
    }
}

/// A compute pipeline for one entry point of composite.wgsl, with its bind group layout.
fn compute_pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    entry_point: &str,
    entries: &[wgpu::BindGroupLayoutEntry],
) -> (wgpu::BindGroupLayout, wgpu::ComputePipeline) {
    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(entry_point),
        entries,
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(entry_point),
        bind_group_layouts: &[Some(&bind_group_layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(entry_point),
        layout: Some(&pipeline_layout),
        module,
        entry_point: Some(entry_point),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });
    (bind_group_layout, pipeline)
}

/// A binding for one tile storage class.
fn tile_binding(binding: u32, format: GpuTileFormat) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: format.binding_type(),
            view_dimension: wgpu::TextureViewDimension::D2Array,
            multisampled: false,
        },
        count: None,
    }
}

/// A transfer function as the (kind, g, a, b) and (c, d, e, f) fields of composite.wgsl.
fn transfer_fields(transfer: TransferFunction) -> ([f32; 4], [f32; 4]) {
    match transfer {
        TransferFunction::Linear => ([0.0; 4], [0.0; 4]),
        TransferFunction::Srgb => ([1.0, 0.0, 0.0, 0.0], [0.0; 4]),
        TransferFunction::Gamma(g) => ([2.0, g, 0.0, 0.0], [0.0; 4]),
        TransferFunction::Rec709 => ([3.0, 0.0, 0.0, 0.0], [0.0; 4]),
        TransferFunction::Parametric {
            g,
            a,
            b,
            c,
            d,
            e,
            f,
        } => ([4.0, g, a, b], [c, d, e, f]),
        TransferFunction::Pq => ([5.0, 0.0, 0.0, 0.0], [0.0; 4]),
        TransferFunction::Hlg => ([6.0, 0.0, 0.0, 0.0], [0.0; 4]),
    }
}

/// Rows of a 3×3 matrix as three padded vec4.
/// The compositing shader, preceded by the constants it shares with this crate and with
/// `slopshop_core::blend`: blend mode numbers, flags and the blend-space matrices.
fn shader_source() -> String {
    let mut constants = String::new();
    for mode in BlendMode::ALL {
        let mut name = String::new();
        for c in mode.id().chars() {
            if c.is_ascii_uppercase() {
                name.push('_');
            }
            name.push(c.to_ascii_uppercase());
        }
        constants += &format!("const MODE_{name}: u32 = {}u;\n", mode.index());
    }
    constants += &format!("const FLAG_PERCEPTUAL: u32 = {FLAG_PERCEPTUAL}u;\n");
    constants += &format!("const BLEND_SHIFT: u32 = {BLEND_SHIFT}u;\n");
    constants += &format!("const FLAG_MASK: u32 = {FLAG_MASK}u;\n");
    constants += &format!("const FLAG_IGNORE_ALPHA: u32 = {FLAG_IGNORE_ALPHA}u;\n");
    constants += &format!("const FLAG_ISOLATED: u32 = {FLAG_ISOLATED}u;\n");
    constants += &format!("const FLAG_ATOP: u32 = {FLAG_ATOP}u;\n");
    constants += &format!("const KIND_GROUP_BEGIN: u32 = {KIND_GROUP_BEGIN}u;\n");
    constants += &format!("const KIND_GROUP_END: u32 = {KIND_GROUP_END}u;\n");
    constants += &format!("const KIND_ADJUST: u32 = {KIND_ADJUST}u;\n");
    constants += &format!(
        "const CURVE_LUT: u32 = {}u;\n",
        slopshop_core::curve::CURVE_LUT
    );
    let [r, g, b] = SRGB_LUMA;
    constants += &format!("const SRGB_LUMA: vec3<f32> = vec3<f32>({r:?}, {g:?}, {b:?});\n");
    constants += &format!("const MAX_GROUP_DEPTH: u32 = {MAX_GROUP_DEPTH}u;\n");
    constants += &format!("const RESAMPLE_NEAREST: u32 = {RESAMPLE_NEAREST}u;\n");
    constants += &format!("const RESAMPLE_EWA: u32 = {RESAMPLE_EWA}u;\n");
    constants += &format!(
        "const EWA_RADIUS2: f32 = {:?};\n",
        (resample::RADIUS * resample::RADIUS) as f32
    );
    constants += &format!("const EWA_TABLE_SIZE: u32 = {}u;\n", resample::TABLE_SIZE);
    constants += &format!(
        "const EWA_TABLE_VEC4S: u32 = {}u;\n",
        resample::TABLE_SIZE / 4
    );
    constants += &format!(
        "const ANTIRING_R2: f32 = {:?};\n",
        resample::ANTIRING_R2 as f32
    );
    constants += &format!(
        "const DIVISION_EPSILON: f32 = {:?};\n",
        slopshop_core::blend::DIVISION_EPSILON as f32
    );
    // The perceptual blend space's primaries are sRGB's (slopshop_core::blend).
    for (prefix, m) in [
        (
            "TO_BLEND",
            WORKING_SPACE.matrix_to(&ColorSpace::LINEAR_SRGB),
        ),
        (
            "FROM_BLEND",
            ColorSpace::LINEAR_SRGB.matrix_to(&WORKING_SPACE),
        ),
    ] {
        for (i, row) in m.iter().enumerate() {
            constants += &format!(
                "const {prefix}{i} = vec3<f32>({:?}, {:?}, {:?});\n",
                row[0] as f32, row[1] as f32, row[2] as f32
            );
        }
    }
    constants + include_str!("composite.wgsl")
}

fn matrix_rows(m: &Mat3) -> [[f32; 4]; 3] {
    m.map(|row| [row[0] as f32, row[1] as f32, row[2] as f32, 0.0])
}

/// Mirrors `Layer` in composite.wgsl.
#[derive(Default)]
struct LayerFields {
    color: [f32; 4],
    kind: u32,
    opacity: f32,
    level_scale: f32,
    table_offset: u32,
    tile_origin: [u32; 2],
    tile_count: [u32; 2],
    level_size: [u32; 2],
    format: u32,
    flags: u32,
    transfer: [f32; 4],
    transfer2: [f32; 4],
    matrix: [[f32; 4]; 3],
    // The enabled mask (FLAG_MASK), sampled like a gray linear raster.
    mask_tile_origin: [u32; 2],
    mask_tile_count: [u32; 2],
    mask_level_size: [u32; 2],
    mask_table_offset: u32,
    mask_level_scale: f32,
    mask_format: u32,
    // Whole-pixel offsets in the document of the raster and of the mask (ADR 0017).
    offset: [i32; 2],
    mask_offset: [i32; 2],
    // Resampling of the raster and of the mask (ADR 0018, `resample_fields`).
    resample: [[f32; 4]; 3],
    mask_resample: [[f32; 4]; 3],
}

impl LayerFields {
    fn write(&self, out: &mut Vec<u8>) {
        let start = out.len();
        for v in self.color {
            out.extend(v.to_le_bytes());
        }
        out.extend(self.kind.to_le_bytes());
        out.extend(self.opacity.to_le_bytes());
        out.extend(self.level_scale.to_le_bytes());
        out.extend(self.table_offset.to_le_bytes());
        for v in self
            .tile_origin
            .iter()
            .chain(&self.tile_count)
            .chain(&self.level_size)
        {
            out.extend(v.to_le_bytes());
        }
        out.extend(self.format.to_le_bytes());
        out.extend(self.flags.to_le_bytes());
        for v in self
            .transfer
            .iter()
            .chain(&self.transfer2)
            .chain(self.matrix.iter().flatten())
        {
            out.extend(v.to_le_bytes());
        }
        for v in self
            .mask_tile_origin
            .iter()
            .chain(&self.mask_tile_count)
            .chain(&self.mask_level_size)
        {
            out.extend(v.to_le_bytes());
        }
        out.extend(self.mask_table_offset.to_le_bytes());
        out.extend(self.mask_level_scale.to_le_bytes());
        out.extend(self.mask_format.to_le_bytes());
        // `offset` is a vec2<i32>: 8-byte aligned.
        out.resize(start + 184, 0);
        for v in self.offset.iter().chain(&self.mask_offset) {
            out.extend(v.to_le_bytes());
        }
        // `resample_u` is a vec4<f32>: 16-byte aligned.
        out.resize(start + 208, 0);
        for v in self.resample.iter().chain(&self.mask_resample).flatten() {
            out.extend(v.to_le_bytes());
        }
        debug_assert_eq!(out.len() - start, LAYER_BYTES);
    }
}

/// Document area covered by the output, clipped to the document: `[x0, y0, x1, y1]`.
fn visible_document_rect(document: Size, view: ViewTransform, output: Size) -> Option<[f64; 4]> {
    let [x0, y0] = view.output_to_document(0.0, 0.0);
    let [x1, y1] = view.output_to_document(f64::from(output.width), f64::from(output.height));
    // The area filter reads half an output pixel beyond the edge pixels' centers.
    let margin = view.scale;
    let x0 = (x0 - margin).max(0.0);
    let y0 = (y0 - margin).max(0.0);
    let x1 = (x1 + margin).min(f64::from(document.width));
    let y1 = (y1 + margin).min(f64::from(document.height));
    (x1 > x0 && y1 > y0).then_some([x0, y0, x1, y1])
}

/// Tiles of a level (`factor` document pixels per level pixel) covering a document area.
fn tile_range(visible: [f64; 4], factor: f64, columns: u32, rows: u32) -> Rect {
    let tile = f64::from(TILE_SIZE) * factor;
    let col0 = ((visible[0] / tile).floor() as u32).min(columns);
    let row0 = ((visible[1] / tile).floor() as u32).min(rows);
    let col1 = ((visible[2] / tile).ceil() as u32).min(columns);
    let row1 = ((visible[3] / tile).ceil() as u32).min(rows);
    Rect::new(
        col0,
        row0,
        col1.saturating_sub(col0),
        row1.saturating_sub(row0),
    )
}

/// Uniform block matching `Params` in `composite.wgsl` (80 bytes, std140-compatible).
fn params_bytes(doc: Size, view: ViewTransform, output: Size, layer_count: u32) -> Vec<u8> {
    // f32 is plenty for display: at 100k px the step is ~0.01 px.
    let mut bytes = Vec::with_capacity(80);
    bytes.extend((view.origin[0] as f32).to_le_bytes());
    bytes.extend((view.origin[1] as f32).to_le_bytes());
    bytes.extend((view.scale as f32).to_le_bytes());
    bytes.extend(layer_count.to_le_bytes());
    for v in [output.width, output.height, doc.width, doc.height] {
        bytes.extend(v.to_le_bytes());
    }
    // The display is sRGB for now (8-bit frames); HDR display comes with ADR 0002's surface.
    let display = matrix_rows(&WORKING_SPACE.matrix_to(&ColorSpace::LINEAR_SRGB));
    for v in display.iter().flatten() {
        bytes.extend(v.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn renderer() -> Option<Renderer> {
        match Renderer::new() {
            Ok(r) => Some(r),
            Err(e) if std::env::var("SLOPSHOP_REQUIRE_GPU").as_deref() == Ok("1") => {
                panic!("GPU required but unavailable: {e}")
            }
            Err(e) => {
                eprintln!("skipping GPU test: {e}");
                None
            }
        }
    }

    fn view(r: &Renderer, document: &Document) -> Result<Frame, RenderError> {
        let output = Size::new(32, 16);
        r.render_view(
            document,
            ViewTransform::fit(document.size(), output, 0),
            output,
        )
    }

    #[test]
    fn device_allows_the_adapters_texture_size() {
        let Some(r) = renderer() else { return };
        // Regression: the baseline limits capped textures at 2048 px, so configuring the
        // surface of a 2560 px wide window panicked.
        assert_eq!(
            r.device.limits().max_texture_dimension_2d,
            r.adapter.limits().max_texture_dimension_2d
        );
    }

    #[test]
    fn gpu_errors_are_captured_not_panics() {
        let Some(r) = renderer() else { return };
        // A buffer beyond the device limit: a validation error, which wgpu's default handler
        // would turn into a panic.
        let too_large = r.max_output_bytes.max(r.device.limits().max_buffer_size) + 4;
        let result = r.capture_errors(|| {
            let _buffer = r.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("too large"),
                size: too_large,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            });
            Ok(())
        });
        assert!(matches!(result, Err(RenderError::Gpu(_))), "{result:?}");
        // Without errors, the result of the work comes through.
        assert_eq!(r.capture_errors(|| Ok(7)).unwrap(), 7);
        // The device still works.
        view(&r, &Document::new(Size::new(64, 32))).unwrap();
    }

    /// Regression: a panic while the tile caches were locked poisoned the lock, and every later
    /// frame failed until the app restarted.
    #[test]
    fn a_poisoned_tile_cache_is_rebuilt() {
        let Some(r) = renderer() else { return };
        let document = Document::new(Size::new(64, 32));
        let before = view(&r, &document).unwrap();
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = r.tile_caches.lock();
            panic!("in a frame");
        }));
        assert!(panicked.is_err());
        assert!(r.tile_caches.is_poisoned());
        assert_eq!(view(&r, &document).unwrap().data, before.data);
        assert!(!r.tile_caches.is_poisoned());
    }
}
