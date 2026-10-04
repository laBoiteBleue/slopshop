//! Filter layers on the display (ADR 0037): what is composited below a filter layer, over what
//! the view shows widened by the filter's reach, at the view's pyramid level (coarser for a far
//! reach), filtered on the GPU and given to the compositor as an image it mixes in
//! (`Step::Filter`). Kept while what is below the layer, its filter and that area are the same:
//! an edit above it costs nothing.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use slopshop_core::color::{AlphaMode, ChannelLayout, PixelFormat, SampleType, WORKING_SPACE};
use slopshop_core::filter::Filter;
use slopshop_core::raster::TILE_SIZE;
use slopshop_core::stack::{FilterStep, Looks, Preview};
use slopshop_core::{Affine, Document, Layer, LayerContent, LayerId, RasterImage, Size};

use crate::tiles::TileCache;
use crate::{Renderer, ViewTransform, WORKGROUP_SIZE, params_bytes, visible_document_rect};

/// The farthest a filter reaches at the level it is computed at, in that level's pixels: beyond,
/// a coarser level (a blur of hundreds of pixels shows no finer detail).
const MAX_LEVEL_REACH: f64 = 128.0;

/// How a filter layer's image is stored: half floats, premultiplied, in the working space (the
/// accumulator's own values, unbounded within the half float range).
const FILTERED_FORMAT: PixelFormat = PixelFormat {
    layout: ChannelLayout::Rgba,
    sample: SampleType::F16,
    color_space: WORKING_SPACE,
    alpha: AlphaMode::Premultiplied,
};

/// Each filter layer's image shown, and the one being computed (ADR 0037).
#[derive(Debug, Default)]
pub(crate) struct FilterLayerImages(Mutex<HashMap<LayerId, Kept>>);

#[derive(Debug, Default)]
struct Kept {
    /// What it was computed from (a hash, see `filter_layer_looks`), and the image.
    shown: Option<(u64, Arc<Preview>)>,
    /// The image being computed, for what.
    job: Option<(u64, Arc<Mutex<Job>>)>,
    /// What an image could not be computed for: not tried again.
    failed: Option<u64>,
}

/// An image computed on a thread of its own, once the GPU is done.
#[derive(Debug)]
enum Job {
    Running,
    Done(Arc<Preview>),
    Failed,
}

impl Renderer {
    /// Add to `looks` the images of the filter layers `view` shows (top level only for now).
    /// One whose layers below, filter or area changed is computed in the background, one at a
    /// time per layer (the latest asked for next), the image computed before shown meanwhile:
    /// whether some image is not the one asked for yet (the display is to be shown again).
    pub(crate) fn filter_layer_looks(
        &self,
        document: &Document,
        view: ViewTransform,
        output: Size,
        caches: &mut [Option<TileCache>; 4],
        looks: &mut Looks,
    ) -> bool {
        let layers = document.layers();
        // The filter layers shown, bottom to top.
        let filters: Vec<(usize, Filter)> = layers
            .iter()
            .enumerate()
            .filter(|(_, layer)| layer.visible && layer.opacity > 0.0)
            .filter_map(|(i, layer)| match layer.content {
                LayerContent::Filter { filter } => Some((i, filter)),
                _ => None,
            })
            .collect();
        if filters.is_empty() {
            return false;
        }
        let Some(visible) = visible_document_rect(document.size(), view, output) else {
            return false;
        };
        let canvas = document.size();
        let next_id = document
            .all_layers()
            .map(|layer| layer.id.get())
            .max()
            .unwrap_or(0)
            + 1;
        let mut images = self
            .filter_layers
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut pending = false;
        for (n, &(index, filter)) in filters.iter().enumerate() {
            let layer = &layers[index];
            // What it and the filter layers above it read around what is shown.
            let margin: f64 = filters[n..].iter().map(|(_, f)| f.reach()).sum();
            let mut level = view.scale.max(1.0).log2().floor() as u32;
            while level < 16 && filter.scaled((1u32 << level) as f32).reach() > MAX_LEVEL_REACH {
                level += 1;
            }
            let f = f64::from(1u32 << level);
            let x0 = ((visible[0] - margin).max(0.0) / f).floor();
            let y0 = ((visible[1] - margin).max(0.0) / f).floor();
            let x1 = ((visible[2] + margin).min(f64::from(canvas.width)) / f).ceil();
            let y1 = ((visible[3] + margin).min(f64::from(canvas.height)) / f).ceil();
            let size = Size::new((x1 - x0) as u32, (y1 - y0) as u32);
            if size.is_empty() {
                continue;
            }
            let below = &layers[..index];
            let key = {
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                hash_layers(below, &mut hasher);
                filter.id().hash(&mut hasher);
                for v in filter.params() {
                    v.to_bits().hash(&mut hasher);
                }
                (level, x0 as u32, y0 as u32, size.width, size.height).hash(&mut hasher);
                document.blend_space().id().hash(&mut hasher);
                hasher.finish()
            };
            let kept = images.entry(layer.id).or_default();
            // A job done: its image shows from now on (if it was for an earlier state, until
            // the next is done).
            if let Some((job_key, job)) = &kept.job {
                let done = match &*job.lock().unwrap_or_else(PoisonError::into_inner) {
                    Job::Running => None,
                    Job::Done(look) => Some(Some(Arc::clone(look))),
                    Job::Failed => Some(None),
                };
                match done {
                    Some(Some(look)) => {
                        kept.shown = Some((*job_key, look));
                        kept.job = None;
                    }
                    Some(None) => {
                        kept.failed = Some(*job_key);
                        kept.job = None;
                    }
                    None => {}
                }
            }
            if kept.shown.as_ref().is_none_or(|(k, _)| *k != key) {
                pending = true;
                if kept.job.is_none() && kept.failed != Some(key) {
                    let level_view = ViewTransform {
                        origin: [x0 * f, y0 * f],
                        scale: f,
                    };
                    let origin = [x0 as u32, y0 as u32];
                    let job = self.start_filtered_below(
                        &prefix_document(document, below, next_id),
                        looks,
                        level_view,
                        size,
                        caches,
                        filter,
                        origin,
                    );
                    kept.job = Some((key, job));
                }
            }
            if let Some((_, look)) = &kept.shown {
                looks.insert(layer.id, Arc::clone(look));
            }
        }
        // Layers gone: their images too.
        images.retain(|id, _| filters.iter().any(|&(i, _)| layers[i].id == *id));
        pending
    }

    /// `prefix` (what is below a filter layer) composited over `view` into an `size` image of
    /// the working space, filtered by `filter` scaled to the view's level: submitted to the GPU,
    /// then read back and made an image (`origin`: its corner, in the level's pixels) on a thread
    /// of its own.
    #[allow(clippy::too_many_arguments)]
    fn start_filtered_below(
        &self,
        prefix: &Option<Document>,
        looks: &Looks,
        view: ViewTransform,
        size: Size,
        caches: &mut [Option<TileCache>; 4],
        filter: Filter,
        origin: [u32; 2],
    ) -> Arc<Mutex<Job>> {
        let job = Arc::new(Mutex::new(Job::Running));
        let readback = prefix.as_ref().and_then(|prefix| {
            self.submit_filtered_below(prefix, looks, view, size, caches, filter)
        });
        let Some(readback) = readback else {
            *job.lock().unwrap_or_else(PoisonError::into_inner) = Job::Failed;
            return job;
        };
        let device = self.gpu_filter.device().clone();
        let level = view.scale;
        let finished = Arc::clone(&job);
        let start = Instant::now();
        let spawned = std::thread::Builder::new()
            .name("filter layer".to_owned())
            .spawn(move || {
                let image = crate::filter::read_floats(&device, &readback).and_then(|floats| {
                    let tiles = half_tiles(&floats, size.width as usize, size.height as usize);
                    RasterImage::from_level0_tiles_only(size, FILTERED_FORMAT, tiles).ok()
                });
                if std::env::var_os("SLOPSHOP_FILTER_TIMING").is_some() {
                    eprintln!(
                        "filter layer {}: {}x{} at {level} document pixels a pixel, {:?}",
                        filter.id(),
                        size.width,
                        size.height,
                        start.elapsed()
                    );
                }
                *finished.lock().unwrap_or_else(PoisonError::into_inner) = match image {
                    Some(image) => Job::Done(Arc::new(Preview {
                        image: Arc::new(image),
                        factor: level as u32,
                        origin,
                        above: Vec::new(),
                        filter: filter.id(),
                    })),
                    None => Job::Failed,
                };
            });
        if spawned.is_err() {
            *job.lock().unwrap_or_else(PoisonError::into_inner) = Job::Failed;
        }
        job
    }

    /// [`Self::start_filtered_below`]'s GPU work: the buffer to read back.
    fn submit_filtered_below(
        &self,
        prefix: &Document,
        looks: &Looks,
        view: ViewTransform,
        size: Size,
        caches: &mut [Option<TileCache>; 4],
        filter: Filter,
    ) -> Option<wgpu::Buffer> {
        use wgpu::util::DeviceExt;
        let bytes = size.pixel_count() * 16;
        if bytes > self.max_output_bytes {
            return None;
        }
        let layers = self.prepare_layers(prefix, looks, view, size, caches);
        let params = params_bytes(prefix.size(), view, size, layers.count, false);
        let params_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("filter layer params"),
                contents: &params,
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let layer_buffers = self.layer_buffers(layers);
        let accumulator = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("filter layer accumulator"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let bind_group = self.bind_group(
            &self.working_bind_group_layout,
            &layer_buffers,
            self.tile_views(caches),
            [
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: accumulator.as_entire_binding(),
                },
            ],
        );
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("filter layer accumulator"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("filter layer accumulator"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.working_pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(
                size.width.div_ceil(WORKGROUP_SIZE),
                size.height.div_ceil(WORKGROUP_SIZE),
                1,
            );
        }
        self.queue.submit([encoder.finish()]);
        // Its pixels to the document: the level's scale, then the view's corner.
        let f = view.scale;
        let step = FilterStep {
            filter: filter.scaled(f as f32),
            selection: None,
            to_document: Affine::scale(f, f)
                .then(Affine::translation(view.origin[0], view.origin[1])),
            space: prefix.blend_space(),
        };
        self.gpu_filter
            .submit_floats(&accumulator, size.width, size.height, &step)
    }
}

/// The document of `below` (what is below a filter layer), on `document`'s canvas.
fn prefix_document(document: &Document, below: &[Layer], next_id: u64) -> Option<Document> {
    Document::restore(
        document.size(),
        WORKING_SPACE,
        document.blend_space(),
        below.to_vec(),
        next_id,
    )
    .ok()
}

/// `pixels` (premultiplied f32 RGBA rows, `width` × `height`) as tiles of half floats, row-major,
/// edge tiles padded by repeating their last row and column as images pad them; tile rows on
/// every core.
fn half_tiles(pixels: &[f32], width: usize, height: usize) -> Vec<Arc<[u8]>> {
    let t = TILE_SIZE as usize;
    let (columns, rows) = (width.div_ceil(t), height.div_ceil(t));
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let per_thread = rows.div_ceil(threads).max(1);
    let tile_rows: Vec<usize> = (0..rows).collect();
    std::thread::scope(|scope| {
        let workers: Vec<_> = tile_rows
            .chunks(per_thread)
            .map(|chunk| {
                scope.spawn(move || {
                    let mut tiles = Vec::with_capacity(chunk.len() * columns);
                    for &row in chunk {
                        for col in 0..columns {
                            let (x0, y0) = (col * t, row * t);
                            let (w, h) = (t.min(width - x0), t.min(height - y0));
                            let mut tile = vec![0u8; t * t * 8];
                            for ty in 0..t {
                                let y = y0 + ty.min(h - 1);
                                for tx in 0..t {
                                    let x = x0 + tx.min(w - 1);
                                    let px = &pixels[(y * width + x) * 4..][..4];
                                    let out = &mut tile[(ty * t + tx) * 8..][..8];
                                    for (c, v) in px.iter().enumerate() {
                                        let half = slopshop_core::color::f32_to_f16(*v);
                                        out[c * 2..c * 2 + 2].copy_from_slice(&half.to_le_bytes());
                                    }
                                }
                            }
                            tiles.push(Arc::<[u8]>::from(tile));
                        }
                    }
                    tiles
                })
            })
            .collect();
        workers
            .into_iter()
            // Invariant: converting floats does not panic.
            .flat_map(|w| w.join().expect("tile worker panicked"))
            .collect()
    })
}

/// What `layers` composite to, as a hash: their order, settings, and contents (pixels by their
/// key, which changes with them).
fn hash_layers(layers: &[Layer], hasher: &mut impl Hasher) {
    layers.len().hash(hasher);
    for layer in layers {
        layer.id.get().hash(hasher);
        layer.visible.hash(hasher);
        layer.opacity.to_bits().hash(hasher);
        layer.blend_mode.index().hash(hasher);
        layer.clipped.hash(hasher);
        for v in layer.transform.to_array() {
            v.to_bits().hash(hasher);
        }
        if let Some(mask) = &layer.mask {
            (mask.enabled, mask.replaces_alpha, mask.image.id().get()).hash(hasher);
        }
        if let Some(style) = &layer.style {
            format!("{:?}", style.settings()).hash(hasher);
        }
        match &layer.content {
            LayerContent::Raster { image, .. } => image.key().hash(hasher),
            LayerContent::Fill { color } => {
                for v in [color.r, color.g, color.b, color.a] {
                    v.to_bits().hash(hasher);
                }
            }
            LayerContent::Group {
                children,
                pass_through,
            } => {
                pass_through.hash(hasher);
                hash_layers(children, hasher);
            }
            LayerContent::Adjustment { adjustment } => {
                adjustment.id().hash(hasher);
                for v in adjustment.params() {
                    v.to_bits().hash(hasher);
                }
            }
            LayerContent::Filter { filter } => {
                filter.id().hash(hasher);
                for v in filter.params() {
                    v.to_bits().hash(hasher);
                }
            }
        }
    }
}
