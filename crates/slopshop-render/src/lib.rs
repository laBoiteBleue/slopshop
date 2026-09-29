//! SlopShop GPU rendering.
//!
//! Renders a view of a document (a region at a given scale) into a display frame. Only the
//! output-sized area is ever computed or read back, never the whole document. Works headless:
//! no window or surface is required.
//!
//! All methods block the calling thread (GPU submission and readback): call them from worker
//! threads, never from a UI thread.

use std::fmt;
use std::sync::mpsc;

use slopshop_core::color::PixelFormat;
use slopshop_core::view::ViewTransform;
use slopshop_core::{Document, LayerContent, Size};

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
    OutputTooLarge { size: Size, max_bytes: u64 },
    Readback(String),
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

/// GPU device plus the viewport compositing pipeline.
#[derive(Debug)]
pub struct Renderer {
    adapter_info: wgpu::AdapterInfo,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    max_output_bytes: u64,
}

const WORKGROUP_SIZE: u32 = 8;
const OUTPUT_FORMAT: PixelFormat = PixelFormat::RGBA8_SRGB;

impl Renderer {
    /// Pick a GPU and build the pipelines. Blocking, and can take a noticeable time.
    pub fn new() -> Result<Self, RenderError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
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
        // (e.g. 8K display) viewports fit in one storage buffer.
        let adapter_limits = adapter.limits();
        let required_limits = wgpu::Limits {
            max_storage_buffer_binding_size: adapter_limits.max_storage_buffer_binding_size,
            max_buffer_size: adapter_limits.max_buffer_size,
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

        let module = device.create_shader_module(wgpu::include_wgsl!("composite.wgsl"));
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
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("composite"),
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
                storage(2, false),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("composite"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("composite"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        let max_output_bytes = required_limits
            .max_storage_buffer_binding_size
            .min(required_limits.max_buffer_size);
        Ok(Self {
            adapter_info: adapter.get_info(),
            device,
            queue,
            pipeline,
            bind_group_layout,
            max_output_bytes,
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
        if output.is_empty() {
            return Err(RenderError::EmptyOutput);
        }
        let byte_len = output.pixel_count() * u64::from(OUTPUT_FORMAT.bytes_per_pixel());
        if byte_len > self.max_output_bytes {
            return Err(RenderError::OutputTooLarge {
                size: output,
                max_bytes: self.max_output_bytes,
            });
        }

        let layers = layer_colors(document);
        let layer_count = (layers.len() / 4) as u32;
        let params = params_bytes(document.size(), view, output, layer_count);

        use wgpu::util::DeviceExt;
        let params_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("composite params"),
                contents: &params,
                usage: wgpu::BufferUsages::UNIFORM,
            });
        // A binding cannot be empty: always upload at least one (unused) entry.
        let layer_bytes: Vec<u8> = if layers.is_empty() {
            vec![0; 16]
        } else {
            layers.iter().flat_map(|v| v.to_le_bytes()).collect()
        };
        let layers_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("composite layers"),
                contents: &layer_bytes,
                usage: wgpu::BufferUsages::STORAGE,
            });
        // Allocated per frame for now; pooling can come once profiling says it matters.
        let output_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("composite output"),
            size: byte_len,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("composite readback"),
            size: byte_len,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("composite"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: layers_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: output_buffer.as_entire_binding(),
                },
            ],
        });

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
        encoder.copy_buffer_to_buffer(&output_buffer, 0, &readback, 0, byte_len);
        self.queue.submit([encoder.finish()]);

        let data = self.read_buffer(&readback)?;
        Ok(Frame {
            size: output,
            format: OUTPUT_FORMAT,
            data,
        })
    }

    /// Map a `MAP_READ` buffer and copy it out. This copy is required: mapped GPU memory cannot
    /// outlive the mapping.
    fn read_buffer(&self, buffer: &wgpu::Buffer) -> Result<Vec<u8>, RenderError> {
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
        let data = slice
            .get_mapped_range()
            .map_err(|e| RenderError::Readback(e.to_string()))?
            .to_vec();
        buffer.unmap();
        Ok(data)
    }
}

/// Visible layers, bottom to top, as premultiplied linear RGBA with opacity applied.
fn layer_colors(document: &Document) -> Vec<f32> {
    document
        .layers()
        .iter()
        .filter(|l| l.visible)
        .flat_map(|l| match l.content {
            LayerContent::Fill { color } => {
                let a = color.a * l.opacity;
                [color.r * a, color.g * a, color.b * a, a]
            }
        })
        .collect()
}

/// Uniform block matching `Params` in `composite.wgsl` (32 bytes, std140-compatible).
fn params_bytes(doc: Size, view: ViewTransform, output: Size, layer_count: u32) -> Vec<u8> {
    // f32 is plenty for display: at 100k px the step is ~0.01 px.
    let mut bytes = Vec::with_capacity(32);
    bytes.extend((view.origin[0] as f32).to_le_bytes());
    bytes.extend((view.origin[1] as f32).to_le_bytes());
    bytes.extend((view.scale as f32).to_le_bytes());
    bytes.extend(layer_count.to_le_bytes());
    for v in [output.width, output.height, doc.width, doc.height] {
        bytes.extend(v.to_le_bytes());
    }
    bytes
}
