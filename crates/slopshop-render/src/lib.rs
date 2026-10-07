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

mod cache;
mod filter;
pub mod present;
mod region;
mod tiles;

pub use region::{export_renderer, export_source};

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError, RwLock, mpsc};
use std::time::{Duration, Instant};

use slopshop_core::adjust::{Adjustment, SRGB_LUMA};
use slopshop_core::color::{
    AlphaMode, ColorSpace, Mat3, PixelFormat, TransferFunction, WORKING_SPACE,
};
use slopshop_core::composite::{Step, display_plan_with, display_steps_with};
use slopshop_core::document::MAX_GROUP_DEPTH;
use slopshop_core::raster::TILE_SIZE;
use slopshop_core::resample::{self, Filter, Resampling};
use slopshop_core::stack::{LookFilter, Looks};
use slopshop_core::tile::TileCoord;
use slopshop_core::view::ViewTransform;
use slopshop_core::{
    Affine, BlendMode, BlendSpace, Document, Layer, LayerContent, Projective, RasterImage, Rect,
    Size,
};

use crate::tiles::{GpuTileFormat, TileCache, TileKey, gpu_texels};

/// A rendered frame, tightly packed rows, top to bottom.
#[derive(Debug, Clone)]
pub struct Frame {
    pub size: Size,
    pub format: PixelFormat,
    pub data: Vec<u8>,
}

/// What a viewport frame cost ([`Renderer::profile_view`]).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FrameStats {
    /// CPU time to plan the frame (steps, pyramid levels) and record its tile uploads.
    pub prepare: Duration,
    /// Steps encoded for the shader, which visits each of them for every pixel.
    pub layers: u32,
    /// Raster tiles uploaded to the GPU (cache misses).
    pub tiles_uploaded: u64,
    /// Display cache (ADR 0022): visible tiles composited for this frame, and those it found
    /// cached. Both 0 when the frame was composited directly.
    pub tiles_composited: u32,
    pub tiles_reused: u32,
    /// Part of the view is shown from a coarser level, its own tiles not composited yet
    /// (progressive frames), or a layer's stack is shown while its pixels are evaluated
    /// (ADR 0029): present again to refine it.
    pub incomplete: bool,
    /// GPU time of the compositing passes, when the adapter supports timestamp queries.
    pub gpu: Option<Duration>,
}

/// The GPU caches of viewport frames, locked together.
#[derive(Debug, Default)]
struct GpuCaches {
    /// Raster tiles, one cache per storage class ([`GpuTileFormat`]), created on first use:
    /// documents without raster layers of a class need no tile memory for it.
    tiles: [Option<TileCache>; 4],
    /// Composited tiles (ADR 0022), created on first use.
    display: Option<cache::DisplayCache>,
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
    /// Display cache (ADR 0022): `fill_main` and `present_main` in composite.wgsl.
    fill_pipeline: wgpu::ComputePipeline,
    fill_bind_group_layout: wgpu::BindGroupLayout,
    present_pipeline: wgpu::ComputePipeline,
    present_bind_group_layout: wgpu::BindGroupLayout,
    /// Quick Mask (ADR 0024): `quick_mask_main` in composite.wgsl, over a finished frame.
    quick_mask_pipeline: wgpu::ComputePipeline,
    quick_mask_bind_group_layout: wgpu::BindGroupLayout,
    /// The marching ants (ADR 0024): `ants_main`, with the same bindings as Quick Mask's pass.
    ants_pipeline: wgpu::ComputePipeline,
    ants_bind_group_layout: wgpu::BindGroupLayout,
    /// Viewport frames go through the display cache (else they composite every visible layer
    /// for every pixel, as before ADR 0022). `SLOPSHOP_DISPLAY_CACHE=0` turns it off.
    use_display_cache: bool,
    /// Evaluate the pixels of shown stacks on threads of their own (ADR 0029); off in tests
    /// that look at the shader's evaluation.
    evaluate_stacks: bool,
    display_capacity: u32,
    caches: Mutex<GpuCaches>,
    tile_capacity: [u32; 4],
    placeholder_tiles: [wgpu::TextureView; 4],
    /// The resampling kernel table (ADR 0018, `resample::weight_table`), as a uniform block.
    ewa_table: wgpu::Buffer,
    /// Nanoseconds per timestamp tick, when the device can time passes ([`FrameStats::gpu`]).
    timestamp_period: Option<f32>,
    /// Filters on the GPU (ADR 0035): the looks at filtered layers the display asks for.
    gpu_filter: Arc<filter::GpuFilter>,
    /// Frames' output buffers, kept for the next frames of the same size (see
    /// [`Self::output_buffer`]).
    outputs: Mutex<Vec<wgpu::Buffer>>,
    /// Shared with [`Self::gpu_filter`]: both submit to the same queue.
    submissions: SubmissionGate,
}

/// Lets work be submitted from any thread, except while one thread needs the queue idle
/// ([`Renderer::without_submissions`]). The lock guards no data: a panic while it was held
/// leaves nothing inconsistent, so poisoning is ignored.
#[derive(Debug, Clone, Default)]
pub(crate) struct SubmissionGate(Arc<RwLock<()>>);

impl SubmissionGate {
    pub(crate) fn submit(
        &self,
        queue: &wgpu::Queue,
        commands: wgpu::CommandBuffer,
    ) -> wgpu::SubmissionIndex {
        let _submitting = self.0.read().unwrap_or_else(PoisonError::into_inner);
        queue.submit([commands])
    }

    fn exclusive<T>(&self, f: impl FnOnce() -> T) -> T {
        let _exclusive = self.0.write().unwrap_or_else(PoisonError::into_inner);
        f()
    }
}

/// What a viewport frame shows over the image: view state, never part of the document or of
/// the display cache.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewOverlays {
    /// Select and Mask's view of the selection, while it is open (over Quick Mask's).
    pub selection_view: SelectionView,
    /// Quick Mask's tint (the document's quick mask, ADR 0024): what it leaves out tinted red
    /// at this opacity, fading where it is soft. Percent, 0–100 (half opaque by default, as Photoshop).
    pub quick_mask_opacity: u8,
    /// The marching ants of the selection, drawn in the frame; `None`: not drawn by the engine
    /// (the UI draws them, or nothing is selected). Never drawn over Quick Mask or Select and
    /// Mask's views, which show the selection themselves.
    pub ants: Option<Ants>,
}

/// The selection's outline drawn over a frame (ADR 0024): one device pixel wide, on the selected
/// pixels of the frame beside an unselected one (coverage crossing one half as the frame's own
/// pixels see it, at any zoom), alternately black and white in dashes of four pixels. The
/// canvas's edge is not an outline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ants {
    /// Where the dashes are, modulo 8: the ants march as it advances ([`Ants::at`]).
    pub phase: u32,
    /// The selection is drawn placed by this map of document pixels: moved by a drag, or
    /// transformed live (Select > Transform Selection), without changing the document.
    pub transform: Affine,
}

impl Ants {
    /// Time for the dashes to advance by one pixel (8 pixels in 0.6 s, as the SVG ants did).
    pub const STEP: Duration = Duration::from_millis(75);
    /// The length of the pattern: dashes of four pixels, dark then light.
    const PERIOD: u32 = 8;

    /// The ants as they are `time` after some fixed instant, marching.
    pub fn at(time: Duration, transform: Affine) -> Self {
        let steps = time.as_millis() / Self::STEP.as_millis();
        Self {
            phase: (steps % u128::from(Self::PERIOD)) as u32,
            transform,
        }
    }

    /// The ants at rest (reduced motion).
    pub fn still(transform: Affine) -> Self {
        Self {
            phase: 0,
            transform,
        }
    }
}

/// How Select and Mask shows the selection over the image (ADR 0024).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SelectionView {
    /// Not shown by this pass (the marching ants, or Quick Mask, show it).
    #[default]
    Off,
    /// Quick Mask's tint over what is left out, at its opacity.
    Overlay,
    /// What is left out black, or white: the selection's content alone.
    OnBlack,
    OnWhite,
    /// The selection itself in gray: white selected, black left out.
    Mask,
}

impl Default for ViewOverlays {
    fn default() -> Self {
        Self {
            selection_view: SelectionView::Off,
            quick_mask_opacity: 50,
            ants: None,
        }
    }
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
/// A stack's original, kept apart from the accumulator (ADR 0029, see `Step::StackOriginal`).
const KIND_STACK_BEGIN: u32 = 5;
/// A stack's paint: the raster is `P`, the mask `k`.
const KIND_STACK_PAINT: u32 = 6;
/// A stack's effect: an adjustment's fields, the selection as the mask.
const KIND_STACK_EFFECT: u32 = 7;
/// The selection pass shows the mask in gray (Select and Mask's Mask view).
const KIND_SHOW_MASK: u32 = 8;
/// A gradient fill layer (`gradient_fill_fields`).
const KIND_GRADIENT: u32 = 9;

/// What the selection pass draws (Quick Mask, Select and Mask's views).
struct SelectionPass<'a> {
    /// `None`: nothing is selected.
    selection: Option<&'a RasterImage>,
    /// The tint over what is left out (encoded display RGB) and its opacity.
    tint: [f32; 3],
    opacity: f32,
    /// The mask in gray instead.
    mask: bool,
    /// The marching ants instead of a tint (`selection` is then placed as they say).
    ants: Option<Ants>,
}
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
/// A raster layer whose pixels are what its stack steps made, not its texels (ADR 0029).
const FLAG_STACK_END: u32 = 128;
/// The layer's blend mode ([`BlendMode::index`]) is stored in the flags from this bit.
const BLEND_SHIFT: u32 = 8;
/// Filters of resampled rasters (`resample_q.w` in composite.wgsl; 0: a whole-pixel offset).
const RESAMPLE_NEAREST: u32 = 1;
const RESAMPLE_EWA: u32 = 2;
/// EWA in perspective (ADR 0038): the ellipse computed at each sample from the map's Jacobian.
const RESAMPLE_PERSPECTIVE: u32 = 3;

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
        // Timestamp queries, where available, only time passes when profiling.
        let timestamps = adapter.features() & wgpu::Features::TIMESTAMP_QUERY;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("slopshop"),
            required_features: timestamps,
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
        let (fill_bind_group_layout, fill_pipeline) = compute_pipeline(
            &device,
            &module,
            "fill_main",
            &[
                &shared[..],
                &[
                    uniform(0),
                    wgpu::BindGroupLayoutEntry {
                        binding: 12,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format: cache::CACHE_FORMAT,
                            view_dimension: wgpu::TextureViewDimension::D2Array,
                        },
                        count: None,
                    },
                    storage(16, true),
                ],
            ]
            .concat(),
        );
        let (present_bind_group_layout, present_pipeline) = compute_pipeline(
            &device,
            &module,
            "present_main",
            &[
                uniform(0),
                storage(2, false),
                uniform(13),
                wgpu::BindGroupLayoutEntry {
                    binding: 14,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                storage(15, true),
            ],
        );

        // The same bindings as `main`: the selection is sampled like a layer's mask.
        let (quick_mask_bind_group_layout, quick_mask_pipeline) = compute_pipeline(
            &device,
            &module,
            "quick_mask_main",
            &[&shared[..], &[uniform(0), storage(2, false)]].concat(),
        );
        let (ants_bind_group_layout, ants_pipeline) = compute_pipeline(
            &device,
            &module,
            "ants_main",
            &[&shared[..], &[uniform(0), storage(2, false)]].concat(),
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
        let timestamp_period = (!timestamps.is_empty()).then(|| queue.get_timestamp_period());
        let submissions = SubmissionGate::default();
        let gpu_filter = Arc::new(filter::GpuFilter::new(&device, &queue, submissions.clone()));
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
            fill_pipeline,
            fill_bind_group_layout,
            present_pipeline,
            present_bind_group_layout,
            quick_mask_pipeline,
            quick_mask_bind_group_layout,
            ants_pipeline,
            ants_bind_group_layout,
            use_display_cache: std::env::var("SLOPSHOP_DISPLAY_CACHE").as_deref() != Ok("0"),
            evaluate_stacks: true,
            display_capacity: cache::cache_capacity(required_limits.max_texture_array_layers),
            max_output_bytes,
            max_dispatch_pixels,
            caches: Mutex::new(GpuCaches::default()),
            outputs: Mutex::new(Vec::new()),
            submissions,
            tile_capacity,
            placeholder_tiles,
            ewa_table,
            timestamp_period,
            gpu_filter,
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
        self.render_view_into(document, view, ViewOverlays::default(), output, &mut data)?;
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

    /// Like [`Self::render_view`] with `overlays` drawn over the image, but appends the RGBA8
    /// sRGB pixels to `out`. Lets callers put a header before the pixels (or reuse an
    /// allocation) without copying the frame again. On error, `out` is left as it was. The
    /// frame's statistics say whether to render it again ([`FrameStats::incomplete`]).
    pub fn render_view_into(
        &self,
        document: &Document,
        view: ViewTransform,
        overlays: ViewOverlays,
        output: Size,
        out: &mut Vec<u8>,
    ) -> Result<FrameStats, RenderError> {
        let options = cache::FrameOptions::default();
        self.read_view_into(document, view, overlays, output, options, out)
    }

    /// Like [`Self::render_view`], but the frame keeps the document's alpha (straight, as a
    /// pixel layer's thumbnail) instead of showing it over the checkerboard, and is transparent
    /// outside the document: for thumbnails of composites.
    pub fn render_view_transparent(
        &self,
        document: &Document,
        view: ViewTransform,
        output: Size,
    ) -> Result<Frame, RenderError> {
        let mut data = Vec::new();
        let options = cache::FrameOptions {
            transparent: true,
            ..cache::FrameOptions::default()
        };
        let overlays = ViewOverlays::default();
        self.read_view_into(document, view, overlays, output, options, &mut data)?;
        Ok(Frame {
            size: output,
            format: OUTPUT_FORMAT,
            data,
        })
    }

    /// [`Self::render_view`] composited progressively, as [`Self::present_view`] does: when
    /// tiles of the display cache are missing, some are composited and the rest is shown from a
    /// coarser level ([`FrameStats::incomplete`]). For tests.
    #[doc(hidden)]
    pub fn render_view_progressive(
        &self,
        document: &Document,
        view: ViewTransform,
        output: Size,
    ) -> Result<(Frame, FrameStats), RenderError> {
        let mut data = Vec::new();
        let overlays = ViewOverlays::default();
        let options = cache::FrameOptions {
            progressive: true,
            ..cache::FrameOptions::default()
        };
        let stats = self.read_view_into(document, view, overlays, output, options, &mut data)?;
        let frame = Frame {
            size: output,
            format: OUTPUT_FORMAT,
            data,
        };
        Ok((frame, stats))
    }

    /// [`Self::render_view_into`] with `options` (progressive, transparent), with the frame's
    /// statistics.
    fn read_view_into(
        &self,
        document: &Document,
        view: ViewTransform,
        overlays: ViewOverlays,
        output: Size,
        options: cache::FrameOptions<'_>,
        out: &mut Vec<u8>,
    ) -> Result<FrameStats, RenderError> {
        let byte_len = self.output_byte_len(output)?;
        let len = out.len();
        self.capture_errors(|| {
            let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("composite readback"),
                size: byte_len,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let pixels = self.output_buffer(byte_len);
            let stats = self.composite(
                document,
                view,
                overlays,
                output,
                options,
                &pixels,
                |encoder, pixels| {
                    encoder.copy_buffer_to_buffer(pixels, 0, &readback, 0, byte_len);
                },
            )?;
            self.keep_output_buffer(pixels);
            self.read_buffer_into(&readback, out)?;
            Ok(stats)
        })
        // A GPU error found after the pixels were appended: leave `out` as it was.
        .inspect_err(|_| out.truncate(len))
    }

    /// Render `view` like [`Self::render_view`] (or progressively like [`Self::present_view`]),
    /// without reading the pixels back, wait for the GPU, and report what the frame cost. For
    /// benchmarks: timing the GPU waits for it, which a frame of the app never does.
    pub fn profile_view(
        &self,
        document: &Document,
        view: ViewTransform,
        output: Size,
        progressive: bool,
    ) -> Result<FrameStats, RenderError> {
        let byte_len = self.output_byte_len(output)?;
        self.capture_errors(|| {
            let queries = self.timestamp_period.map(|_| {
                self.device.create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some("composite timestamps"),
                    ty: wgpu::QueryType::Timestamp,
                    count: 2,
                })
            });
            let buffer = |label, usage| {
                self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size: 16,
                    usage,
                    mapped_at_creation: false,
                })
            };
            let resolved = buffer(
                "timestamps resolved",
                wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            );
            let readback = buffer(
                "timestamps readback",
                wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            );
            let options = cache::FrameOptions {
                timestamps: queries.as_ref(),
                progressive,
                transparent: false,
            };
            let pixels = self.output_buffer(byte_len);
            let mut stats = self.composite(
                document,
                view,
                ViewOverlays::default(),
                output,
                options,
                &pixels,
                |encoder, _| {
                    if let Some(queries) = &queries {
                        encoder.resolve_query_set(queries, 0..2, &resolved, 0);
                        encoder.copy_buffer_to_buffer(&resolved, 0, &readback, 0, 16);
                    }
                },
            )?;
            self.keep_output_buffer(pixels);
            // Waits for the frame, timed or not.
            let mut bytes = Vec::new();
            self.read_buffer_into(&readback, &mut bytes)?;
            if let Some(period) = self.timestamp_period {
                let tick = |at: usize| {
                    bytes
                        .get(at..at + 8)
                        .and_then(|b| b.try_into().ok())
                        .map_or(0, u64::from_le_bytes)
                };
                let ticks = tick(8).saturating_sub(tick(0));
                stats.gpu = Some(Duration::from_secs_f64(
                    ticks as f64 * f64::from(period) * 1e-9,
                ));
            }
            Ok(stats)
        })
    }

    /// A buffer for a frame's pixels (`byte_len` bytes): one a frame of the same size left, else
    /// a new one. Give it back with [`Self::keep_output_buffer`] once the frame is submitted (the
    /// queue runs a later frame's writes after the earlier frame's reads).
    fn output_buffer(&self, byte_len: u64) -> wgpu::Buffer {
        let mut outputs = self
            .outputs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match outputs.iter().position(|b| b.size() == byte_len) {
            Some(at) => outputs.swap_remove(at),
            None => self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("composite output"),
                size: byte_len,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
        }
    }

    /// Keep a frame's output buffer for the next frames, the few most recent sizes only (a
    /// viewport, a navigator, the frames over IPC).
    fn keep_output_buffer(&self, buffer: wgpu::Buffer) {
        const KEPT: usize = 4;
        let mut outputs = self
            .outputs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if outputs.len() >= KEPT {
            outputs.remove(0);
        }
        outputs.push(buffer);
    }

    /// Submit `commands` to the queue; waits while a window surface is configured
    /// ([`Self::without_submissions`]). Every submission goes through here or the gate.
    fn submit(&self, commands: wgpu::CommandBuffer) -> wgpu::SubmissionIndex {
        self.submissions.submit(&self.queue, commands)
    }

    /// Run `f` while no other thread submits work. Configuring a window surface waits for the
    /// GPU to be idle, then fails ("Failed to wait for GPU to come idle") if work was submitted
    /// meanwhile, as when the display cache fills from other threads after an image opens.
    fn without_submissions<T>(&self, f: impl FnOnce() -> T) -> T {
        self.submissions.exclusive(f)
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
    /// to a surface…), and submit. Does not wait for the GPU. With `options.timestamps` (2
    /// queries), the passes write their start and end there.
    #[allow(clippy::too_many_arguments)]
    fn composite(
        &self,
        document: &Document,
        view: ViewTransform,
        overlays: ViewOverlays,
        output: Size,
        options: cache::FrameOptions<'_>,
        output_buffer: &wgpu::Buffer,
        finish: impl FnOnce(&mut wgpu::CommandEncoder, &wgpu::Buffer),
    ) -> Result<FrameStats, RenderError> {
        // Rejected before the caches are locked (and reset on error).
        self.output_byte_len(output)?;

        // The cache lock is held until the GPU work is submitted: tiles resident for this frame
        // must not be evicted before.
        let mut cache_guard = self.caches.lock().unwrap_or_else(|poisoned| {
            // A panic while the caches were locked may have left them inconsistent: start
            // afresh rather than failing every later frame.
            let mut guard = poisoned.into_inner();
            *guard = GpuCaches::default();
            self.caches.clear_poison();
            guard
        });
        let result = self.capture_errors(|| {
            self.composite_locked(
                document,
                view,
                overlays,
                output,
                options,
                output_buffer,
                &mut cache_guard,
                finish,
            )
        });
        if result.is_err() {
            // Tiles may have been recorded as resident in a cache whose texture, upload or fill
            // failed: rebuild the caches on the next frame.
            *cache_guard = GpuCaches::default();
        }
        result
    }

    /// [`Self::composite`] with the tile caches locked.
    #[allow(clippy::too_many_arguments)]
    fn composite_locked(
        &self,
        document: &Document,
        view: ViewTransform,
        overlays: ViewOverlays,
        output: Size,
        options: cache::FrameOptions<'_>,
        output_buffer: &wgpu::Buffer,
        caches: &mut GpuCaches,
        finish: impl FnOnce(&mut wgpu::CommandEncoder, &wgpu::Buffer),
    ) -> Result<FrameStats, RenderError> {
        let timestamps = options.timestamps;
        let start = Instant::now();
        // Stacks not evaluated yet are shown by the shader, close to their pixels (exact but for
        // rounding at 100 %, evaluated on coarser levels when zoomed out): their pixels are
        // evaluated meanwhile, and the frame asks to be shown again until they are (ADR 0029).
        // Likewise layer styles' effects, computed in the background while what they drew last
        // shows (ADR 0032), and the looks at filtered layers (ADR 0034).
        let gpu = Arc::clone(&self.gpu_filter);
        let gpu: LookFilter = Arc::new(move |job| gpu.look(job));
        let (looks, looks_pending) = gather_looks(document, view, output, &gpu);
        let pending =
            start_stack_evaluations(document, &looks, self.evaluate_stacks) || looks_pending;
        let uploads = |caches: &GpuCaches| -> u64 {
            caches.tiles.iter().flatten().map(TileCache::uploads).sum()
        };
        // A cache created by this frame starts from zero.
        let uploaded_before = uploads(caches);
        let mut stats = FrameStats::default();

        if self.use_display_cache {
            let cached = self.composite_cached(
                document,
                &looks,
                view,
                output,
                output_buffer,
                options,
                caches,
            );
            if let Some((encoder, cached_stats)) = cached {
                stats = cached_stats;
                stats.incomplete |= pending;
                stats.prepare = start.elapsed();
                stats.tiles_uploaded = uploads(caches).saturating_sub(uploaded_before);
                let frame = Composited {
                    document,
                    view,
                    output,
                    pixels: output_buffer,
                };
                self.finish_frame(encoder, &frame, overlays, &mut caches.tiles, finish);
                return Ok(stats);
            }
        }

        let layers = self.prepare_layers(document, &looks, view, output, &mut caches.tiles);
        stats.incomplete = pending;
        stats.prepare = start.elapsed();
        stats.layers = layers.count;
        stats.tiles_uploaded = uploads(caches).saturating_sub(uploaded_before);
        let params = params_bytes(
            document.size(),
            view,
            output,
            layers.count,
            options.transparent,
        );

        use wgpu::util::DeviceExt;
        let params_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("composite params"),
                contents: &params,
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let layer_buffers = self.layer_buffers(layers);

        let bind_group = self.bind_group(
            &self.bind_group_layout,
            &layer_buffers,
            self.tile_views(&caches.tiles),
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
                timestamp_writes: timestamps.map(|query_set| wgpu::ComputePassTimestampWrites {
                    query_set,
                    beginning_of_pass_write_index: Some(0),
                    end_of_pass_write_index: Some(1),
                }),
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(
                output.width.div_ceil(WORKGROUP_SIZE),
                output.height.div_ceil(WORKGROUP_SIZE),
                1,
            );
        }
        let frame = Composited {
            document,
            view,
            output,
            pixels: output_buffer,
        };
        self.finish_frame(encoder, &frame, overlays, &mut caches.tiles, finish);
        Ok(stats)
    }

    /// Draw `overlays` over a composited frame (view state: never in the display cache), let
    /// `finish` record what to do with its pixels, and submit.
    fn finish_frame(
        &self,
        mut encoder: wgpu::CommandEncoder,
        frame: &Composited<'_>,
        overlays: ViewOverlays,
        tiles: &mut [Option<TileCache>; 4],
        finish: impl FnOnce(&mut wgpu::CommandEncoder, &wgpu::Buffer),
    ) {
        let opacity = f32::from(overlays.quick_mask_opacity.min(100)) / 100.0;
        // What the pass draws: a tint and its opacity, or the mask itself.
        let quick_mask = frame.document.quick_mask();
        let shown = match overlays.selection_view {
            SelectionView::Off => quick_mask.map(|_| ([1.0, 0.0, 0.0], opacity, false)),
            SelectionView::Overlay => Some(([1.0, 0.0, 0.0], opacity, false)),
            SelectionView::OnBlack => Some(([0.0; 3], 1.0, false)),
            SelectionView::OnWhite => Some(([1.0; 3], 1.0, false)),
            SelectionView::Mask => Some(([0.0; 3], 1.0, true)),
        };
        // Quick Mask shows its own image; Select and Mask's views the selection (without one,
        // nothing is left selected).
        let selection = match overlays.selection_view {
            SelectionView::Off => quick_mask,
            _ => frame.document.selection(),
        };
        let pass = match shown {
            Some((tint, opacity, mask)) => Some(SelectionPass {
                selection: selection.map(|s| s.image().as_ref()),
                tint,
                opacity,
                mask,
                ants: None,
            }),
            // The ants, where nothing else shows the selection (none selected: none drawn).
            None => overlays
                .ants
                .zip(frame.document.selection())
                .map(|(ants, selection)| SelectionPass {
                    selection: Some(selection.image().as_ref()),
                    tint: [0.0; 3],
                    opacity: 1.0,
                    mask: false,
                    ants: Some(ants),
                }),
        };
        if let Some(pass) = pass {
            // The tiles the frame reads stay resident only until it is submitted: submit it
            // first, so that the overlay's uploads cannot replace them under it.
            self.submit(encoder.finish());
            encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("selection overlay"),
                });
            self.record_selection_pass(&mut encoder, frame, pass, tiles);
        }
        finish(&mut encoder, frame.pixels);
        self.submit(encoder.finish());
    }

    /// Quick Mask and Select and Mask's views (ADR 0024): the unselected area of the frame
    /// tinted, or the selection shown in gray; or the marching ants: the selection sampled at
    /// the view's level like a layer's mask.
    fn record_selection_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &Composited<'_>,
        pass: SelectionPass<'_>,
        tiles: &mut [Option<TileCache>; 4],
    ) {
        let doc_size = frame.document.size();
        let placed_by = pass.ants.map_or(Affine::IDENTITY, |ants| ants.transform);
        let plan = pass.selection.and_then(|selection| {
            visible_document_rect(doc_size, frame.view, frame.output).and_then(|visible| {
                RasterPlan::new(selection, visible, placed_by.into(), frame.view.scale)
            })
        });
        let mut prepared = PreparedLayers {
            count: 1,
            bytes: Vec::new(),
            tile_table: Vec::new(),
        };
        // A mask with no tile in view (or none planned) reads 0: all of the view is tinted.
        let [r, g, b] = pass.tint;
        let mut fields = LayerFields {
            kind: if pass.mask { KIND_SHOW_MASK } else { KIND_FILL },
            color: [r, g, b, 1.0],
            opacity: pass.opacity,
            ..LayerFields::default()
        };
        if let Some(ants) = pass.ants {
            // `ants_main` reads its phase (a small whole number, exact as a float) there.
            fields.color = [(ants.phase % Ants::PERIOD) as f32, 0.0, 0.0, 1.0];
        }
        if let Some(plan) = plan.filter(|plan| !plan.range().is_empty()) {
            let capacity = self.tile_capacity[plan.format.index()];
            let cache = tiles[plan.format.index()]
                .get_or_insert_with(|| TileCache::new(&self.device, plan.format, capacity));
            cache.begin_frame();
            let slots = self.upload(&plan, cache);
            set_mask_fields(&mut fields, &plan, &mut prepared.tile_table, slots);
        }
        fields.write(&mut prepared.bytes);
        let params = params_bytes(doc_size, frame.view, frame.output, 1, false);
        use wgpu::util::DeviceExt;
        let params_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("quick mask params"),
                contents: &params,
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let layer_buffers = self.layer_buffers(prepared);
        let (pipeline, layout) = if pass.ants.is_some() {
            (&self.ants_pipeline, &self.ants_bind_group_layout)
        } else {
            (
                &self.quick_mask_pipeline,
                &self.quick_mask_bind_group_layout,
            )
        };
        let bind_group = self.bind_group(
            layout,
            &layer_buffers,
            self.tile_views(tiles),
            [
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: frame.pixels.as_entire_binding(),
                },
            ],
        );
        let mut compute = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("selection overlay"),
            timestamp_writes: None,
        });
        compute.set_pipeline(pipeline);
        compute.set_bind_group(0, &bind_group, &[]);
        compute.dispatch_workgroups(
            frame.output.width.div_ceil(WORKGROUP_SIZE),
            frame.output.height.div_ceil(WORKGROUP_SIZE),
            1,
        );
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
        wait_mapped(&self.device, &rx).map_err(RenderError::Readback)?;
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

/// Wait until the mapping that `rx` is told the result of is done, polling the device a moment
/// at a time: a blocking poll would hold the device's lock until the GPU is done with all its
/// work, and a native present waits for that lock meanwhile (wgpu-core's snatch lock).
pub(crate) fn wait_mapped(
    device: &wgpu::Device,
    rx: &mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
) -> Result<(), String> {
    loop {
        device
            .poll(wgpu::PollType::Poll)
            .map_err(|e| e.to_string())?;
        match rx.try_recv() {
            Ok(mapped) => return mapped.map_err(|e| e.to_string()),
            Err(mpsc::TryRecvError::Empty) => std::thread::sleep(Duration::from_micros(200)),
            Err(mpsc::TryRecvError::Disconnected) => return Err("the mapping went away".into()),
        }
    }
}

/// Size of one `Layer` in composite.wgsl.
const LAYER_BYTES: usize = 304;

/// GPU-ready description of the visible layers of one frame.
/// A composited frame waiting for its overlays and its `finish`.
struct Composited<'a> {
    document: &'a Document,
    view: ViewTransform,
    output: Size,
    /// The frame's packed RGBA8 sRGB pixels.
    pixels: &'a wgpu::Buffer,
}

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
        looks: &Looks,
        view: ViewTransform,
        output: Size,
        caches: &mut [Option<TileCache>; 4],
    ) -> PreparedLayers {
        let visible_doc = visible_document_rect(document.size(), view, output);
        // Stacks not evaluated yet are evaluated by the shader (ADR 0029), those with a filter
        // over the look at what is shown (ADR 0034).
        let steps = display_steps_with(document, Some(looks));

        // Plan the pyramid level of every raster layer together, so that all visible tiles
        // fit in the cache: coarser levels rather than missing layers.
        // Two plans per step: its raster, then its enabled mask (ADR 0014), planned together.
        // None for a raster without visible tiles: nothing of it to sample in this view, so the
        // shader does not visit it for every pixel.
        let mut plans: Vec<Option<RasterPlan<'_>>> = steps
            .iter()
            .flat_map(|step| {
                step_rasters(step).map(|raster| {
                    let (image, transform) = raster?;
                    RasterPlan::new(image, visible_doc?, transform, view.scale)
                        .filter(|plan| !plan.range().is_empty())
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

/// Start evaluating, each on a thread of its own, the pixels of the shown layers whose stack
/// the display evaluates (ADR 0029); whether there are any, or layer style effects shown that
/// are not drawn yet (planning the display started them, ADR 0032). A stack with a filter is
/// not evaluated whole for the display: only its quick look is started, and the look at what
/// is shown (see [`gather_looks`]) tells whether the frame must be shown again (ADR 0034).
fn start_stack_evaluations(document: &Document, looks: &Looks, start: bool) -> bool {
    let (steps, mut pending) = display_plan_with(document, Some(looks));
    for step in steps {
        if let Step::Layer { layer, stack, .. } = step
            && let LayerContent::Raster {
                image,
                stack: layer_stack,
                ..
            } = &layer.content
            && (stack || image.ready_image().is_none())
        {
            if start {
                image.evaluate_in_background();
            }
            pending |= !layer_stack.as_ref().is_some_and(|s| s.has_shown_filter());
        }
    }
    pending
}

/// What the display shows of the shown layers whose stack has a filter and is not evaluated
/// (ADR 0034): the look at the part of each in `view`, at the level it is seen at, computed on
/// a thread of its own when it is not there yet (meanwhile, the quick look); and whether the
/// frame must be shown again for one.
fn gather_looks(
    document: &Document,
    view: ViewTransform,
    output: Size,
    gpu: &LookFilter,
) -> (Looks, bool) {
    let mut looks = Looks::new();
    let mut pending = false;
    let Some(visible) = visible_document_rect(document.size(), view, output) else {
        return (looks, false);
    };
    for layer in document.all_layers().filter(|l| l.visible) {
        let LayerContent::Raster {
            image,
            stack: Some(stack),
            ..
        } = &layer.content
        else {
            continue;
        };
        if !stack.has_shown_filter() || image.ready_image().is_some() {
            continue;
        }
        let to_document = layer.transform.then(document.parent_transform(layer.id));
        let Some(to_layer) = to_document.inverse() else {
            continue;
        };
        // Pixels of the layer per screen pixel (the view's scale is document pixels per screen
        // pixel): the level the display samples it at; a projective layer's, where the view's
        // middle is.
        let [vx0, vy0, vx1, vy1] = visible;
        let (cx, cy) = to_layer.apply((vx0 + vx1) / 2.0, (vy0 + vy1) / 2.0);
        let [a, b, c, d] = to_document.jacobian(cx, cy);
        let scale = (a * d - b * c).abs().sqrt();
        let per_screen = view.scale / scale;
        let level = if per_screen > 1.0 {
            per_screen.log2().floor() as usize
        } else {
            0
        };
        let (look, again) = image.look_for(to_layer.map_rect(visible), level, Some(gpu));
        pending |= again;
        if let Some(look) = look {
            looks.insert(layer.id, look);
        }
    }
    (looks, pending)
}

/// The rasters a step samples, with their transforms to the document: a raster layer's image,
/// then its (or a group's) enabled mask.
fn step_rasters<'a>(step: &Step<'a>) -> [Option<(&'a RasterImage, Projective)>; 2] {
    match step {
        Step::Layer {
            layer,
            transform,
            stack,
            ..
        } => {
            let content = match &layer.content {
                // Pixels made by the stack steps before it (ADR 0029): planned as its original,
                // which tells where they are; not sampled.
                LayerContent::Raster {
                    stack: Some(layer_stack),
                    ..
                } if *stack => Some((layer_stack.original().as_ref(), *transform)),
                // Its pixels, or what the layer showed before while they are evaluated.
                LayerContent::Raster { image, .. } => {
                    image.shown().map(|image| (image.as_ref(), *transform))
                }
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
        Step::StackOriginal {
            original,
            transform,
        } => [Some((*original, *transform)), None],
        // `P`, then `k` read as a mask: both in the layer's pixels.
        Step::StackPaint { paint, transform } => match paint.image_refs() {
            Some((color, keep)) => [
                Some((color.as_ref(), *transform)),
                Some((keep.as_ref(), *transform)),
            ],
            None => [None, None],
        },
        // The selection, at the document's pixels when the effect was applied: from there to
        // the layer's pixels then, then to the document now.
        Step::StackEffect { effect, transform } => [
            None,
            effect.selection.as_ref().and_then(|selection| {
                let placed = effect.to_document.inverse()?.then(*transform);
                Some((selection.image().as_ref(), placed))
            }),
        ],
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
    // Whether the stack steps under way are encoded (their original is in this view).
    let mut stack_shown = false;
    for (i, step) in steps.iter().enumerate() {
        let (plan, mask_plan) = (&plans[2 * i], &plans[2 * i + 1]);
        let table = tables.next().unwrap_or_default();
        let mask_table = tables.next().unwrap_or_default();
        let (layer, mode, opacity, atop, stacked) = match step {
            Step::Layer {
                layer,
                mode,
                opacity,
                atop,
                stack,
                ..
            } => (*layer, *mode, *opacity, *atop, *stack),
            Step::StackOriginal { .. } => {
                // Nothing of the layer in this view: its stack steps are left out with it.
                let Some(plan) = plan else {
                    stack_shown = false;
                    continue;
                };
                stack_shown = true;
                let mut fields = LayerFields {
                    kind: KIND_STACK_BEGIN,
                    ..LayerFields::default()
                };
                set_raster_fields(&mut fields, plan, &mut prepared.tile_table, table);
                fields.write(&mut prepared.bytes);
                prepared.count += 1;
                continue;
            }
            Step::StackPaint { paint, .. } => {
                let (true, Some(color), Some(keep)) = (stack_shown, plan, mask_plan) else {
                    continue;
                };
                let mut fields = LayerFields {
                    kind: KIND_STACK_PAINT,
                    flags: if paint.space() == BlendSpace::Perceptual {
                        FLAG_PERCEPTUAL
                    } else {
                        0
                    },
                    ..LayerFields::default()
                };
                set_raster_fields(&mut fields, color, &mut prepared.tile_table, table);
                set_mask_fields(&mut fields, keep, &mut prepared.tile_table, mask_table);
                fields.write(&mut prepared.bytes);
                prepared.count += 1;
                continue;
            }
            Step::StackEffect { effect, .. } => {
                // Its selection has nothing in this view: it changes nothing here.
                if !stack_shown || (effect.selection.is_some() && mask_plan.is_none()) {
                    continue;
                }
                let perceptual = if effect.space == BlendSpace::Perceptual {
                    FLAG_PERCEPTUAL
                } else {
                    0
                };
                let mut fields = adjustment_fields(
                    &effect.adjustment,
                    1.0,
                    perceptual,
                    &mut prepared.tile_table,
                );
                fields.kind = KIND_STACK_EFFECT;
                if let Some(mask) = mask_plan {
                    set_mask_fields(&mut fields, mask, &mut prepared.tile_table, mask_table);
                }
                fields.write(&mut prepared.bytes);
                prepared.count += 1;
                continue;
            }
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
                let mut fields =
                    adjustment_fields(adjustment, *opacity, perceptual, &mut prepared.tile_table);
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
            LayerContent::GradientFill { field } => {
                let Step::Layer { transform, .. } = step else {
                    continue;
                };
                // A layer placed where nothing can be seen (its transform not invertible).
                let Some(to_content) = transform.inverse() else {
                    continue;
                };
                let alpha = if replaces_alpha {
                    [1.0; 2]
                } else {
                    field.alpha
                };
                set_gradient_fields(
                    &mut fields,
                    field,
                    to_content,
                    alpha,
                    &mut prepared.tile_table,
                );
                fields.opacity = opacity;
            }
            LayerContent::Raster { .. } => {
                // Not visible in this view or region: nothing to sample.
                let Some(plan) = plan else { continue };
                if stacked && !stack_shown {
                    continue;
                }
                fields.kind = KIND_RASTER;
                fields.opacity = opacity;
                set_raster_fields(&mut fields, plan, &mut prepared.tile_table, table);
                if replaces_alpha {
                    fields.flags |= FLAG_IGNORE_ALPHA;
                }
                // Its pixels are what its stack steps made (ADR 0029).
                if stacked {
                    fields.flags |= FLAG_STACK_END;
                }
            }
            // Groups and adjustments are steps of their own; a vector layer is drawn as its
            // paints.
            LayerContent::Group { .. }
            | LayerContent::Adjustment { .. }
            | LayerContent::Vector { .. } => continue,
        }
        if let Some(mask) = mask_plan {
            set_mask_fields(&mut fields, mask, &mut prepared.tile_table, mask_table);
        }
        fields.write(&mut prepared.bytes);
        prepared.count += 1;
    }
    prepared
}

/// Describe a gradient fill in `fields`: `color`, its ends (`from`, `to`) in the layer's
/// content space; `resample_u`, `resample_v` and `resample_q`, the rows of the map from document
/// points there (projective: divided by the third, ADR 0038); `transfer`,
/// its opacity at each end and its shape; `matrix`, from linear sRGB to the working space; its
/// lookup tables (as Gradient Map's, `adjustment_table`) appended to `tile_table`.
fn set_gradient_fields(
    fields: &mut LayerFields,
    field: &slopshop_core::gradient::GradientField,
    to_content: Projective,
    alpha: [f32; 2],
    tile_table: &mut Vec<u32>,
) {
    fields.kind = KIND_GRADIENT;
    let [a, b, c, d, e, f, g, h, i] = to_content.to_array().map(|v| v as f32);
    fields.resample[0] = [a, c, e, 0.0];
    fields.resample[1] = [b, d, f, 0.0];
    fields.resample[2] = [g, h, i, 0.0];
    fields.color = [field.from[0], field.from[1], field.to[0], field.to[1]].map(|v| v as f32);
    let shape = match field.shape {
        slopshop_core::gradient::GradientShape::Linear => 0.0,
        slopshop_core::gradient::GradientShape::Radial => 1.0,
    };
    fields.transfer = [alpha[0], alpha[1], shape, 0.0];
    fields.matrix = matrix_rows(&ColorSpace::LINEAR_SRGB.matrix_to(&WORKING_SPACE));
    fields.table_offset = tile_table.len() as u32;
    tile_table.extend_from_slice(&adjustment_table(&Adjustment::GradientMap {
        gradient: field.gradient,
        reverse: false,
    }));
}

/// Describe a raster's plan in `fields` (where it is, how its texels decode), its tile slots
/// appended to `tile_table`.
fn set_raster_fields(
    fields: &mut LayerFields,
    plan: &RasterPlan<'_>,
    tile_table: &mut Vec<u32>,
    slots: Vec<u32>,
) {
    fields.offset = plan.offset();
    fields.resample = resample_fields(plan);
    let range = plan.range();
    let size = plan.image.levels()[plan.level].size();
    fields.level_scale = plan.factor() as f32;
    fields.table_offset = tile_table.len() as u32;
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
    tile_table.extend(slots);
}

/// The fields of an adjustment (an adjustment layer at `opacity`, or a stack's effect): its
/// 20 parameters in fields it has no other use for, Curves' lookup tables appended to
/// `tile_table`.
fn adjustment_fields(
    adjustment: &Adjustment,
    opacity: f32,
    flags: u32,
    tile_table: &mut Vec<u32>,
) -> LayerFields {
    let mut p = adjustment.params();
    if let Adjustment::PhotoFilter { color, .. } = adjustment {
        // The shader multiplies by the linear sRGB color.
        let filter = slopshop_core::adjust::filter_color(*color);
        p[..3].copy_from_slice(&filter.map(|v| v as f32));
    }
    let vec4 = |at: usize| [p[at], p[at + 1], p[at + 2], p[at + 3]];
    // Selective Color reads its parameters from the tile table (below); the others fit in
    // the first 20.
    let mut fields = LayerFields {
        kind: KIND_ADJUST,
        flags,
        opacity,
        format: adjustment.index(),
        color: vec4(0),
        transfer: vec4(4),
        transfer2: vec4(8),
        matrix: [vec4(12), vec4(16), [0.0; 4]],
        ..LayerFields::default()
    };
    // Its table, after the tile slots.
    let table = adjustment_table(adjustment);
    if !table.is_empty() {
        fields.table_offset = tile_table.len() as u32;
        tile_table.extend_from_slice(&table);
    }
    fields
}

/// What an adjustment reads from the tile table: Curves' lookup tables (composite, red, green,
/// blue), Selective Color's parameters (more than the layer fields hold), Gradient Map's lookup
/// tables (red, green, blue); nothing for the others. Made once per adjustment and kept while it
/// is used: a frame encodes the adjustment for each tile it composites, and a slider dragged
/// gives a new one at each frame.
fn adjustment_table(adjustment: &Adjustment) -> Arc<[u32]> {
    type Made = Vec<(Adjustment, Arc<[u32]>)>;
    /// The tables made last, the most recent first.
    static MADE: std::sync::Mutex<Made> = std::sync::Mutex::new(Vec::new());
    const KEPT: usize = 16;
    let tabled = adjustment.curves().is_some()
        || adjustment.gradient().is_some()
        || matches!(adjustment, Adjustment::SelectiveColor { .. });
    if !tabled {
        return Arc::from([]);
    }
    let mut made = MADE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(at) = made.iter().position(|(a, _)| a == adjustment) {
        let entry = made.remove(at);
        let table = Arc::clone(&entry.1);
        made.insert(0, entry);
        return table;
    }
    let mut table = Vec::new();
    if let Some(curves) = adjustment.curves() {
        for curve in &curves {
            table.extend(curve.lut().iter().map(|v| v.to_bits()));
        }
    }
    if let Adjustment::SelectiveColor { .. } = adjustment {
        table.extend(adjustment.params().iter().map(|v| v.to_bits()));
    }
    if let Some(gradient) = adjustment.gradient() {
        for lut in gradient.luts() {
            table.extend(lut.iter().map(|v| v.to_bits()));
        }
    }
    let table: Arc<[u32]> = table.into();
    made.truncate(KEPT - 1);
    made.insert(0, (*adjustment, Arc::clone(&table)));
    table
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
/// in the first two rows, the ellipse's quadratic form and the filter in the third. In
/// perspective (ADR 0038), the three rows of the projective map, the output scale in the
/// first's `w` (the shader computes each sample's ellipse).
fn resample_fields(plan: &RasterPlan<'_>) -> [[f32; 4]; 3] {
    let Some(r) = plan.resampling() else {
        return [[0.0; 4]; 3];
    };
    if let Some(p) = r.perspective() {
        let [a, b, c, d, e, f, g, h, i] = p.to_texel.to_array().map(|v| v as f32);
        return [
            [a, c, e, r.scale() as f32],
            [b, d, f, 0.0],
            [g, h, i, RESAMPLE_PERSPECTIVE as f32],
        ];
    }
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
    /// Make the planned tiles resident; returns their cache slots (row-major), `None` when the
    /// cache has no slot left for this frame.
    fn try_upload(&self, plan: &RasterPlan<'_>, cache: &mut TileCache) -> Option<Vec<u32>> {
        let stored = plan.image.stored_format();
        plan.keys()
            .map(|key| match key {
                Some(key) => {
                    let tile = Arc::clone(&key.0);
                    cache.ensure(&self.queue, key, || gpu_texels(&tile, stored))
                }
                // Not stored: transparent.
                None => Some(NO_TILE),
            })
            .collect()
    }

    /// Make the planned tiles resident; returns their cache slots (row-major).
    fn upload(&self, plan: &RasterPlan<'_>, cache: &mut TileCache) -> Vec<u32> {
        let stored = plan.image.stored_format();
        plan.keys()
            .map(|key| {
                key.and_then(|key| {
                    let tile = Arc::clone(&key.0);
                    cache.ensure(&self.queue, key, || gpu_texels(&tile, stored))
                })
                .unwrap_or(NO_TILE)
            })
            .collect()
    }

    /// Limit every tile cache to `capacity` tiles (at least 1). For tests and
    /// memory-constrained setups; resets the caches.
    #[doc(hidden)]
    pub fn with_tile_capacity(mut self, capacity: u32) -> Self {
        self.tile_capacity = self.tile_capacity.map(|c| capacity.clamp(1, c.max(1)));
        self.caches = Mutex::new(GpuCaches::default());
        self
    }

    /// Composite viewport frames through the display cache (ADR 0022) or directly. For tests
    /// and comparisons; resets the caches.
    #[doc(hidden)]
    /// Whether the pixels of the shown stacks are evaluated on threads of their own while the
    /// shader shows them (ADR 0029). For tests of the shader's evaluation.
    pub fn with_stack_evaluation(mut self, enabled: bool) -> Self {
        self.evaluate_stacks = enabled;
        self
    }

    pub fn with_display_cache(mut self, enabled: bool) -> Self {
        self.use_display_cache = enabled;
        self.caches = Mutex::new(GpuCaches::default());
        self
    }

    /// Limit the display cache to `capacity` tiles (at least 1). For tests; resets the caches.
    #[doc(hidden)]
    pub fn with_display_cache_capacity(mut self, capacity: u32) -> Self {
        self.display_capacity = capacity.clamp(1, self.display_capacity);
        self.caches = Mutex::new(GpuCaches::default());
        self
    }
}

/// How one visible raster layer is sampled in a frame.
#[derive(Clone, Copy)]
struct RasterPlan<'a> {
    image: &'a RasterImage,
    format: GpuTileFormat,
    /// Document area to cover: `[x0, y0, x1, y1]`.
    area: [f64; 4],
    place: Place,
    level: usize,
}

/// Where a planned raster is in the document.
// A few plans per frame, copied freely: a perspective's numbers inline cost less than a box.
#[allow(clippy::large_enum_variant)]
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
    fn new(
        image: &'a RasterImage,
        area: [f64; 4],
        transform: Projective,
        scale: f64,
    ) -> Option<Self> {
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
                let r = Resampling::placed(transform, scale, image.levels().len(), image.size())?;
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
    fn full_resolution(
        image: &'a RasterImage,
        area: [f64; 4],
        transform: Projective,
    ) -> Option<Self> {
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
        self.range_over(self.area)
    }

    /// The tiles of the planned level that the document `area` reads (the plan's own area
    /// aside): the placed image's, and for a resampled one its filter's support.
    fn range_over(&self, area: [f64; 4]) -> Rect {
        let grid = self.image.levels()[self.level].grid();
        // The image's area (level-0 pixels) that the document area reads.
        let visible = match self.resampling() {
            Some(r) => r.source_area(area),
            None => shifted(area, self.offset()),
        };
        tile_range(visible, self.factor(), grid.columns(), grid.rows())
    }

    /// The tiles of `range` (at the planned level), row-major, borrowed (`None` for one that is
    /// not stored).
    fn tile_refs(&self, range: Rect) -> impl Iterator<Item = Option<&'a Arc<[u8]>>> + use<'a> {
        let level = &self.image.levels()[self.level];
        (range.y..range.y + range.height).flat_map(move |row| {
            (range.x..range.x + range.width).map(move |col| level.tile(TileCoord { col, row }))
        })
    }

    /// [`Self::tile_refs`] with `visit` called on each tile: faster for what runs per tile
    /// of a frame, which is most of the time.
    fn for_each_tile(&self, range: Rect, mut visit: impl FnMut(Option<&Arc<[u8]>>)) {
        let level = &self.image.levels()[self.level];
        for row in range.y..range.y + range.height {
            for col in range.x..range.x + range.width {
                visit(level.tile(TileCoord { col, row }));
            }
        }
    }

    /// The visible tiles, row-major (`None` for one that is not stored).
    fn keys(&self) -> impl Iterator<Item = Option<TileKey>> + use<'a> {
        self.tile_refs(self.range())
            .map(|tile| tile.map(|tile| TileKey(Arc::clone(tile))))
    }
}

/// Coarsen plans until the distinct visible tiles of each storage class fit in that class's
/// cache: each step coarsens the plan of an over-budget class that needs the most tiles. Layers
/// sharing an image at the same level count once.
fn fit_tile_budget(plans: &mut [Option<RasterPlan<'_>>], capacity: [u32; 4]) {
    for format in GpuTileFormat::ALL {
        // How many plans read each tile: the distinct tiles are its keys. Updated for the plan
        // coarsened at each step, not recounted.
        let mut readers: HashMap<TileKey, u32> = HashMap::new();
        for plan in plans.iter().flatten().filter(|p| p.format == format) {
            for key in plan.keys().flatten() {
                *readers.entry(key).or_default() += 1;
            }
        }
        while readers.len() > capacity[format.index()] as usize {
            let largest = plans
                .iter_mut()
                .flatten()
                .filter(|plan| plan.format == format && plan.can_coarsen())
                .max_by_key(|plan| plan.range().width * plan.range().height);
            // All at their coarsest level: the upload order decides what is left out.
            let Some(plan) = largest else { break };
            for key in plan.keys().flatten() {
                if let Some(n) = readers.get_mut(&key) {
                    *n -= 1;
                    if *n == 0 {
                        readers.remove(&key);
                    }
                }
            }
            plan.level += 1;
            for key in plan.keys().flatten() {
                *readers.entry(key).or_default() += 1;
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
    constants += &format!("const KIND_STACK_BEGIN: u32 = {KIND_STACK_BEGIN}u;\n");
    constants += &format!("const KIND_STACK_PAINT: u32 = {KIND_STACK_PAINT}u;\n");
    constants += &format!("const KIND_STACK_EFFECT: u32 = {KIND_STACK_EFFECT}u;\n");
    constants += &format!("const KIND_SHOW_MASK: u32 = {KIND_SHOW_MASK}u;\n");
    constants += &format!("const KIND_GRADIENT: u32 = {KIND_GRADIENT}u;\n");
    constants += &format!("const FLAG_STACK_END: u32 = {FLAG_STACK_END}u;\n");
    constants += &format!(
        "const CURVE_LUT: u32 = {}u;\n",
        slopshop_core::curve::CURVE_LUT
    );
    let [r, g, b] = SRGB_LUMA;
    constants += &format!("const SRGB_LUMA: vec3<f32> = vec3<f32>({r:?}, {g:?}, {b:?});\n");
    constants += &format!("const MAX_GROUP_DEPTH: u32 = {MAX_GROUP_DEPTH}u;\n");
    constants += &format!("const RESAMPLE_NEAREST: u32 = {RESAMPLE_NEAREST}u;\n");
    constants += &format!("const RESAMPLE_EWA: u32 = {RESAMPLE_EWA}u;\n");
    constants += &format!("const RESAMPLE_PERSPECTIVE: u32 = {RESAMPLE_PERSPECTIVE}u;\n");
    constants += &format!(
        "const EWA_MAX_EXTENT: f32 = {:?};\n",
        resample::MAX_EXTENT as f32
    );
    constants += &format!("const EWA_RADIUS: f32 = {:?};\n", resample::RADIUS as f32);
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

/// Uniform block matching `Params` in `composite.wgsl` (96 bytes, std140-compatible).
/// `transparent`: the frame keeps the document's alpha (thumbnails), no checkerboard.
fn params_bytes(
    doc: Size,
    view: ViewTransform,
    output: Size,
    layer_count: u32,
    transparent: bool,
) -> Vec<u8> {
    // f32 is plenty for display: at 100k px the step is ~0.01 px.
    let mut bytes = Vec::with_capacity(96);
    bytes.extend((view.origin[0] as f32).to_le_bytes());
    bytes.extend((view.origin[1] as f32).to_le_bytes());
    bytes.extend((view.scale as f32).to_le_bytes());
    bytes.extend(layer_count.to_le_bytes());
    for v in [output.width, output.height, doc.width, doc.height] {
        bytes.extend(v.to_le_bytes());
    }
    for v in display_matrix().iter().flatten() {
        bytes.extend(v.to_le_bytes());
    }
    bytes.extend(u32::from(transparent).to_le_bytes());
    // The struct's size rounds up to its 16-byte alignment.
    bytes.resize(96, 0);
    bytes
}

/// Working space → display, rows of a 3×3 matrix. The display is sRGB for now (8-bit frames);
/// HDR display comes with ADR 0002's surface.
fn display_matrix() -> [[f32; 4]; 3] {
    matrix_rows(&WORKING_SPACE.matrix_to(&ColorSpace::LINEAR_SRGB))
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

    #[test]
    fn an_adjustment_s_table_is_made_once_and_holds_what_the_shader_reads() {
        use slopshop_core::curve::Curve;
        let curve = Curve::new(&[[0, 0], [90, 140], [255, 255]]).unwrap();
        let curves = Adjustment::Curves {
            rgb: curve,
            red: Curve::IDENTITY,
            green: Curve::IDENTITY,
            blue: curve,
        };
        let table = adjustment_table(&curves);
        let lut = curve.lut();
        assert_eq!(table.len(), 4 * lut.len());
        let bits: Vec<u32> = lut.iter().map(|v| v.to_bits()).collect();
        assert_eq!(&table[..lut.len()], &bits[..]);
        assert_eq!(&table[3 * lut.len()..], &bits[..]);
        // The same adjustment again (the next tile, the next frame): the same table.
        assert!(Arc::ptr_eq(&table, &adjustment_table(&curves)));
        // Another one: its own.
        let other = Adjustment::Curves {
            rgb: Curve::IDENTITY,
            red: curve,
            green: Curve::IDENTITY,
            blue: Curve::IDENTITY,
        };
        let theirs = adjustment_table(&other);
        assert_eq!(&theirs[lut.len()..2 * lut.len()], &bits[..]);
        // Adjustments without a table have none.
        assert!(adjustment_table(&Adjustment::Invert).is_empty());
    }

    #[test]
    fn frames_of_a_size_reuse_their_output_buffer() {
        let Some(r) = renderer() else { return };
        let first = r.output_buffer(4096);
        let id = first.clone();
        r.keep_output_buffer(first);
        // Another size: a buffer of its own.
        let other = r.output_buffer(8192);
        assert_eq!(other.size(), 8192);
        let again = r.output_buffer(4096);
        assert!(again == id);
        r.keep_output_buffer(other);
        r.keep_output_buffer(again);
        // A few sizes are kept, the oldest dropped.
        for size in [1024, 2048, 3072, 5120] {
            let buffer = r.output_buffer(size);
            r.keep_output_buffer(buffer);
        }
        assert_eq!(r.outputs.lock().unwrap().len(), 4);
        assert!(r.output_buffer(4096) != id);
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

    /// Regression: configuring the window surface failed ("Failed to wait for GPU to come idle")
    /// when another thread submitted work meanwhile, as the display cache did after an image
    /// opened. Surfaces need a window: this checks that submissions wait for the configuration.
    #[test]
    fn submissions_wait_while_a_surface_is_configured() {
        let Some(r) = renderer() else { return };
        let (sent, received) = mpsc::channel();
        std::thread::scope(|scope| {
            let submitted_meanwhile = r.without_submissions(|| {
                scope.spawn(|| {
                    let encoder = r
                        .device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
                    r.submit(encoder.finish());
                    sent.send(()).expect("the test waits for the submission");
                });
                received.recv_timeout(Duration::from_millis(200)).is_ok()
            });
            assert!(
                !submitted_meanwhile,
                "work was submitted during the configuration"
            );
            received
                .recv_timeout(Duration::from_secs(10))
                .expect("the submission goes through once the surface is configured");
        });
    }

    /// Regression: a panic while the tile caches were locked poisoned the lock, and every later
    /// frame failed until the app restarted.
    #[test]
    fn a_poisoned_tile_cache_is_rebuilt() {
        let Some(r) = renderer() else { return };
        let document = Document::new(Size::new(64, 32));
        let before = view(&r, &document).unwrap();
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = r.caches.lock();
            panic!("in a frame");
        }));
        assert!(panicked.is_err());
        assert!(r.caches.is_poisoned());
        assert_eq!(view(&r, &document).unwrap().data, before.data);
        assert!(!r.caches.is_poisoned());
    }

    #[test]
    fn transparent_frames_keep_the_document_s_alpha() {
        let Some(r) = renderer() else { return };
        let mut document = Document::new(Size::new(16, 16));
        let format = slopshop_core::color::PixelFormat::RGBA8_SRGB;
        // Straight orange at alpha 128 (sRGB-encoded samples).
        let image =
            RasterImage::from_pixels(Size::new(16, 16), format, &[240, 120, 30, 128].repeat(256))
                .expect("16 × 16 RGBA8 pixels");
        let layer = Layer {
            style: None,
            transform: Affine::IDENTITY.into(),
            clipped: false,
            id: document.allocate_layer_id(),
            name: "image".into(),
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask: None,
            content: LayerContent::Raster {
                source: None,
                stack: None,
                image: slopshop_core::stack::Pixels::ready(image.into()),
            },
        };
        slopshop_core::Edit::InsertLayer {
            parent: None,
            index: 0,
            layer,
        }
        .apply(&mut document)
        .expect("a valid layer");
        // Four pixels to the left of the document, then the document.
        let view = ViewTransform {
            origin: [-4.0, 0.0],
            scale: 1.0,
        };
        let frame = r
            .render_view_transparent(&document, view, Size::new(12, 4))
            .expect("a frame");
        let pixel = |x: usize| &frame.data[x * 4..x * 4 + 4];
        assert_eq!(pixel(0), [0, 0, 0, 0]);
        let inside = pixel(8);
        assert!(inside[3].abs_diff(128) <= 1, "{inside:?}");
        for (got, want) in inside[..3].iter().zip([240u8, 120, 30]) {
            assert!(got.abs_diff(want) <= 2, "{inside:?}");
        }
        // The view itself still shows it over the checkerboard, opaque.
        let shown = r
            .render_view(&document, view, Size::new(12, 4))
            .expect("a frame");
        assert_eq!(shown.data[8 * 4 + 3], 255);
    }

    #[test]
    fn rasters_outside_the_view_are_not_encoded() {
        let Some(r) = renderer() else { return };
        let mut document = Document::new(Size::new(1024, 1024));
        let format = slopshop_core::color::PixelFormat::RGBA8_SRGB;
        let image = RasterImage::from_pixels(Size::new(16, 16), format, &[200; 16 * 16 * 4])
            .expect("16 × 16 RGBA8 pixels");
        let layer = Layer {
            style: None,
            transform: Affine::translation(8.0, 8.0).into(),
            clipped: false,
            id: document.allocate_layer_id(),
            name: "image".into(),
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask: None,
            content: LayerContent::Raster {
                source: None,
                stack: None,
                image: slopshop_core::stack::Pixels::ready(image.into()),
            },
        };
        slopshop_core::Edit::InsertLayer {
            parent: None,
            index: 0,
            layer,
        }
        .apply(&mut document)
        .expect("a valid layer");
        let output = Size::new(64, 64);
        let encoded = |origin: [f64; 2]| {
            let mut caches = r.caches.lock().expect("not poisoned");
            let view = ViewTransform { origin, scale: 1.0 };
            r.prepare_layers(&document, &Looks::new(), view, output, &mut caches.tiles)
                .count
        };
        assert_eq!(encoded([0.0, 0.0]), 1);
        assert_eq!(encoded([512.0, 512.0]), 0);
    }

    #[test]
    fn profiled_frames_report_their_layers_and_uploads() {
        let Some(r) = renderer() else { return };
        let mut document = Document::new(Size::new(600, 300));
        let format = slopshop_core::color::PixelFormat::RGBA8_SRGB;
        let image = RasterImage::from_pixels(Size::new(600, 300), format, &[90; 600 * 300 * 4])
            .expect("600 × 300 RGBA8 pixels");
        let layer = Layer {
            style: None,
            transform: Affine::IDENTITY.into(),
            clipped: false,
            id: document.allocate_layer_id(),
            name: "image".into(),
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask: None,
            content: LayerContent::Raster {
                source: None,
                stack: None,
                image: slopshop_core::stack::Pixels::ready(image.into()),
            },
        };
        slopshop_core::Edit::InsertLayer {
            parent: None,
            index: 0,
            layer,
        }
        .apply(&mut document)
        .expect("a valid layer");
        let view = ViewTransform {
            origin: [0.0, 0.0],
            scale: 1.0,
        };
        let output = Size::new(600, 300);
        let first = r.profile_view(&document, view, output, false).unwrap();
        assert_eq!(first.layers, 1);
        // 600 × 300 pixels at level 0: 3 × 2 tiles, uploaded once.
        assert_eq!(first.tiles_uploaded, 6);
        let again = r.profile_view(&document, view, output, false).unwrap();
        assert_eq!(again.tiles_uploaded, 0);
        assert_eq!(again.gpu.is_some(), r.timestamp_period.is_some());

        // A document whose image shares five of these six tiles (a painted one, ADR 0027):
        // only the new tile is uploaded.
        let LayerContent::Raster { image, .. } = &document.layers()[0].content else {
            panic!("a raster layer");
        };
        let image = image.get();
        let mut tiles = image.levels()[0].tiles().to_vec();
        tiles[4] = vec![7u8; tiles[4].len()].into();
        let painted = RasterImage::from_level0_tiles(image.size(), image.format(), tiles)
            .expect("the same grid");
        let mut repainted = Document::new(document.size());
        let layer = Layer {
            id: repainted.allocate_layer_id(),
            content: LayerContent::Raster {
                source: None,
                stack: None,
                image: slopshop_core::stack::Pixels::ready(painted.into()),
            },
            ..document.layers()[0].clone()
        };
        slopshop_core::Edit::InsertLayer {
            parent: None,
            index: 0,
            layer,
        }
        .apply(&mut repainted)
        .expect("a valid layer");
        let after = r.profile_view(&repainted, view, output, false).unwrap();
        assert_eq!(after.tiles_uploaded, 1);
    }

    #[test]
    fn the_gpu_places_mosaic_cells_from_the_layers_origin_in_a_crop() {
        use slopshop_core::BlendSpace;
        use slopshop_core::filter::Filter;
        use slopshop_core::stack::{FilterStep, LayerStack};
        let Some(r) = renderer() else { return };
        let size = Size::new(600, 120);
        let bytes: Vec<u8> = (0..size.height)
            .flat_map(|y| {
                (0..size.width).flat_map(move |x| [(x % 251) as u8, (y * 2) as u8, 90, 255])
            })
            .collect();
        let image = Arc::new(
            RasterImage::from_pixels(size, slopshop_core::color::PixelFormat::RGBA8_SRGB, &bytes)
                .unwrap(),
        );
        let stack = LayerStack::new(Arc::clone(&image))
            .with_filter(
                FilterStep {
                    filter: Filter::Mosaic { cell: 30.0 },
                    selection: None,
                    to_document: Affine::IDENTITY.into(),
                    space: BlendSpace::Perceptual,
                },
                Some(Arc::clone(&image)),
            )
            .unwrap();
        // The right part only: the crop starts at the second column of tiles.
        let job = stack.look_job([300.0, 10.0, 590.0, 110.0], 0).unwrap();
        assert_eq!(job.origin, [256, 0]);
        let gpu = r.gpu_filter.look(&job).expect("taken by the GPU");
        let cpu = job.run().unwrap().image;
        let (a, b) = (gpu.levels()[0].tiles(), cpu.levels()[0].tiles());
        let worst = a
            .iter()
            .zip(b)
            .flat_map(|(s, t)| s.iter().zip(t.iter()).map(|(x, y)| x.abs_diff(*y)))
            .max()
            .unwrap_or(0);
        assert!(worst <= 1, "{worst}");
    }

    #[test]
    fn the_gpu_filters_a_look_as_the_cpu_does() {
        use slopshop_core::BlendSpace;
        use slopshop_core::filter::Filter;
        use slopshop_core::stack::{FilterStep, LayerStack};
        let Some(r) = renderer() else { return };
        // A sharp edge with some transparency, over two tiles and a half.
        let size = Size::new(600, 300);
        let mut bytes = Vec::new();
        for y in 0..size.height {
            for x in 0..size.width {
                let v = if x < 290 { 20 } else { 230 };
                let a = if y < 40 { 0 } else { 255 };
                bytes.extend([v, (y % 256) as u8, 128, a]);
            }
        }
        let original = Arc::new(
            RasterImage::from_pixels(size, slopshop_core::color::PixelFormat::RGBA8_SRGB, &bytes)
                .unwrap(),
        );
        // The same pixels opaque, as an RGB layer (a JPEG's) stores them: RGBA, alpha opaque.
        let rgb_bytes: Vec<u8> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect();
        let rgb = Arc::new(
            RasterImage::from_pixels(
                size,
                slopshop_core::color::PixelFormat {
                    layout: slopshop_core::color::ChannelLayout::Rgb,
                    ..slopshop_core::color::PixelFormat::RGBA8_SRGB
                },
                &rgb_bytes,
            )
            .unwrap(),
        );
        let blur = |radius| Filter::GaussianBlur { radius };
        let sharpen = Filter::UnsharpMask {
            amount: 150.0,
            radius: 2.0,
            threshold: 0.0,
        };
        // The CPU's three boxes beyond a radius of 8 are a little off the exact Gaussian; Unsharp
        // Mask amplifies its blur's rounding.
        let filters = [
            (blur(1.5), 1),
            (blur(6.0), 1),
            (blur(20.0), 4),
            (sharpen, 2),
            (Filter::HighPass { radius: 4.0 }, 1),
            (
                Filter::MotionBlur {
                    angle: 30.0,
                    distance: 25.0,
                },
                1,
            ),
            (
                Filter::MotionBlur {
                    angle: -90.0,
                    distance: 7.5,
                },
                1,
            ),
            (
                Filter::AddNoise {
                    amount: 40.0,
                    gaussian: false,
                    monochromatic: false,
                    seed: 12,
                },
                1,
            ),
            (
                Filter::AddNoise {
                    amount: 25.0,
                    gaussian: true,
                    monochromatic: true,
                    seed: 99,
                },
                1,
            ),
            (
                Filter::DustAndScratches {
                    radius: 3.0,
                    threshold: 0.0,
                },
                1,
            ),
            (
                Filter::DustAndScratches {
                    radius: 1.0,
                    threshold: 20.0,
                },
                1,
            ),
            (Filter::Median { radius: 2.0 }, 1),
            (Filter::BoxBlur { radius: 6.0 }, 1),
            (Filter::BoxBlur { radius: 40.0 }, 1),
            (Filter::Maximum { radius: 3.0 }, 0),
            (Filter::Minimum { radius: 9.0 }, 0),
            (Filter::Solarize, 1),
            (Filter::FindEdges, 1),
            (
                Filter::Emboss {
                    angle: 135.0,
                    height: 3.0,
                    amount: 150.0,
                },
                1,
            ),
            (Filter::Mosaic { cell: 17.0 }, 1),
            (
                Filter::Offset {
                    horizontal: 37.0,
                    vertical: -12.0,
                    edge: slopshop_core::filter::OffsetEdge::Wrap,
                },
                0,
            ),
            (
                Filter::Offset {
                    horizontal: -50.0,
                    vertical: 20.0,
                    edge: slopshop_core::filter::OffsetEdge::Transparent,
                },
                0,
            ),
            (Filter::Twirl { angle: 200.0 }, 2),
            (Filter::Pinch { amount: 60.0 }, 2),
            (Filter::Spherize { amount: -70.0 }, 2),
            (Filter::PolarCoordinates { to_polar: true }, 2),
            (Filter::PolarCoordinates { to_polar: false }, 2),
            // The CPU's broad blur is three boxes: a little off the GPU's exact one.
            (
                Filter::ClarityTexture {
                    texture: 60.0,
                    clarity: -80.0,
                    scale: 1.0,
                },
                3,
            ),
        ];
        for (image, (filter, most)) in [&original, &rgb]
            .into_iter()
            .flat_map(|image| filters.iter().map(move |f| (image, *f)))
        {
            let stack = LayerStack::new(Arc::clone(image))
                .with_filter(
                    FilterStep {
                        filter,
                        selection: None,
                        to_document: Affine::IDENTITY.into(),
                        space: BlendSpace::Perceptual,
                    },
                    Some(Arc::clone(image)),
                )
                .unwrap();
            let job = stack.look_job([200.0, 60.0, 400.0, 250.0], 0).unwrap();
            // Offset's transparent edge on a layer without transparency: the CPU's.
            let transparent = matches!(
                filter,
                Filter::Offset {
                    edge: slopshop_core::filter::OffsetEdge::Transparent,
                    ..
                }
            );
            if transparent && !image.format().layout.has_alpha() {
                assert!(r.gpu_filter.look(&job).is_none());
                continue;
            }
            let gpu = r.gpu_filter.look(&job).expect("taken by the GPU");
            let cpu = job.run().unwrap().image;
            assert_eq!((gpu.size(), gpu.format()), (cpu.size(), cpu.format()));
            let (a, b) = (gpu.levels()[0].tiles(), cpu.levels()[0].tiles());
            let worst = a
                .iter()
                .zip(b)
                .flat_map(|(s, t)| s.iter().zip(t.iter()).map(|(x, y)| x.abs_diff(*y)))
                .max()
                .unwrap_or(0);
            let format = image.format().layout;
            assert!(worst <= most, "{format:?} {filter:?}: {worst}");
            // RGB stays opaque, as its tiles must.
            if format == slopshop_core::color::ChannelLayout::Rgb {
                assert!(
                    a.iter()
                        .all(|t| t.as_chunks::<4>().0.iter().all(|p| p[3] == 255)),
                    "{filter:?}"
                );
            }
        }
        // Not taken: a selection, or another format (the CPU computes those).
        let gray = Arc::new(
            RasterImage::from_pixels(
                size,
                slopshop_core::color::PixelFormat {
                    layout: slopshop_core::color::ChannelLayout::Gray,
                    ..slopshop_core::color::PixelFormat::RGBA8_SRGB
                },
                &vec![9; (size.width * size.height) as usize],
            )
            .unwrap(),
        );
        let stack = LayerStack::new(Arc::clone(&gray))
            .with_filter(
                FilterStep {
                    filter: Filter::GaussianBlur { radius: 2.0 },
                    selection: None,
                    to_document: Affine::IDENTITY.into(),
                    space: BlendSpace::Perceptual,
                },
                Some(gray),
            )
            .unwrap();
        let job = stack.look_job([0.0, 0.0, 100.0, 100.0], 0).unwrap();
        assert!(r.gpu_filter.look(&job).is_none());
        // A Liquify look is warped on the CPU, even in the format the GPU takes (ADR 0037).
        let liquified = LayerStack::new(Arc::clone(&original))
            .with_liquify(
                Arc::new(slopshop_core::liquify::Field::new(size)),
                BlendSpace::Perceptual,
                Some(Arc::clone(&original)),
            )
            .unwrap();
        let job = liquified.look_job([0.0, 0.0, 100.0, 100.0], 0).unwrap();
        assert!(job.warp.is_some() && r.gpu_filter.look(&job).is_none());
    }

    /// Timing of the looks at a filtered layer a 1080p view asks for, on the GPU and on the CPU
    /// (run with `--ignored --nocapture`): a slider dragged asks for one per setting.
    #[test]
    #[ignore = "benchmark"]
    fn bench_gpu_looks() {
        use slopshop_core::BlendSpace;
        use slopshop_core::filter::Filter;
        use slopshop_core::stack::{FilterStep, LayerStack};
        let Some(r) = renderer() else { return };
        let size = Size::new(6000, 4000);
        let mut bytes = Vec::with_capacity(size.pixel_count() as usize * 4);
        for y in 0..size.height {
            for x in 0..size.width {
                bytes.extend([(x % 251) as u8, (y % 241) as u8, ((x ^ y) % 256) as u8, 255]);
            }
        }
        let original = Arc::new(
            RasterImage::from_pixels(size, slopshop_core::color::PixelFormat::RGBA8_SRGB, &bytes)
                .unwrap(),
        );
        for filter in [
            Filter::GaussianBlur { radius: 4.0 },
            Filter::UnsharpMask {
                amount: 120.0,
                radius: 3.0,
                threshold: 0.0,
            },
            Filter::DustAndScratches {
                radius: 2.0,
                threshold: 0.0,
            },
        ] {
            let stack = LayerStack::new(Arc::clone(&original))
                .with_filter(
                    FilterStep {
                        filter,
                        selection: None,
                        to_document: Affine::IDENTITY.into(),
                        space: BlendSpace::Perceptual,
                    },
                    Some(Arc::clone(&original)),
                )
                .unwrap();
            let job = stack.look_job([2000.0, 1500.0, 3920.0, 2580.0], 0).unwrap();
            let time = |f: &dyn Fn()| {
                let mut runs: Vec<f64> = (0..15)
                    .map(|_| {
                        let start = Instant::now();
                        f();
                        start.elapsed().as_secs_f64() * 1000.0
                    })
                    .collect();
                runs.sort_by(f64::total_cmp);
                runs[runs.len() / 2]
            };
            let gpu = time(&|| {
                r.gpu_filter.look(&job).expect("taken by the GPU");
            });
            let cpu = time(&|| {
                job.run().unwrap();
            });
            println!(
                "{} look of {}x{}: GPU {gpu:.1} ms, CPU {cpu:.1} ms (medians)",
                filter.id(),
                job.size.width,
                job.size.height
            );
        }
    }
}
