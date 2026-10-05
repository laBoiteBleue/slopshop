//! Full-resolution rendering of document regions, for export (ADR 0008).
//!
//! Same compositing code as the viewport (`export_main` in composite.wgsl), but the output is
//! the working-space values themselves: premultiplied RGBA f32, one level-0 texel per pixel,
//! with no display conversion, background or clipping. A region is rendered in chunks when its
//! tiles do not fit in the tile cache or its pixels in the chunk budget; the result is the same.

use std::collections::HashSet;
use std::sync::mpsc;

use slopshop_core::composite::{Step, steps};
use slopshop_core::raster::{ImageId, TILE_SIZE};
use slopshop_core::resample::Resampling;
use slopshop_core::{BlendSpace, Document, Projective, RasterImage, Rect};

use crate::tiles::{GpuTileFormat, TileCache, TileKey};
use crate::{
    NO_TILE, RasterPlan, RenderError, Renderer, WORKGROUP_SIZE, encode_layers, step_rasters,
};

/// One RGBA f32 pixel.
const PIXEL_BYTES: u64 = 16;

/// Most bytes of one chunk's output buffer (its readback buffer is as large). The device's
/// binding limit (up to 2 GiB) says what can be bound, not what VRAM has room for next to the
/// viewport's caches: export must not assume a large region fits (ADR 0008).
const CHUNK_BUDGET_BYTES: u64 = 128 * 1024 * 1024;

/// The non-finite counter of `export_main`: two u32 words (low, high).
const COUNTER_BYTES: u64 = 8;

/// Pixel source for export (`slopshop_io::export::export_image`): full-resolution regions of
/// `document`, rendered by `renderer`, or by the CPU reference compositor
/// (`slopshop_core::composite`) when there is no renderer. Both give the same values (within
/// float rounding) and return the number of non-finite values they replaced (NaN and ±inf
/// samples; see [`Renderer::render_region`]).
///
/// GPU errors that the CPU can work around fall back to it (see [`on_gpu_error`]): a region
/// that needs more tile slots than the GPU cache has ([`RenderError::TooManyLayers`]) is
/// rendered on the CPU, and after an out-of-memory or other device error
/// ([`RenderError::OutOfMemory`], [`RenderError::Gpu`]) every remaining region is. Other errors
/// (e.g. a failed readback) are reported.
pub fn export_source<'a>(
    renderer: Option<&'a Renderer>,
    document: &'a Document,
) -> impl FnMut(Rect, &mut [f32]) -> Result<u64, String> + Send + 'a {
    let mut render = export_renderer(renderer);
    move |region, out| render(document, region, out)
}

/// [`export_source`] for several documents (`slopshop_io::export::export_psd` renders each
/// layer as its own document): the chunk buffers and the CPU fallback last across documents.
pub fn export_renderer(
    renderer: Option<&Renderer>,
) -> impl FnMut(&Document, Rect, &mut [f32]) -> Result<u64, String> + Send + '_ {
    // The chunk buffers are kept across regions (bands): allocated once per export.
    let mut gpu = renderer.map(|renderer| (renderer, RegionBuffers::default()));
    move |document, region, out| {
        if let Some((renderer, buffers)) = &mut gpu {
            match renderer.render_region_with(document, region, out, buffers) {
                Ok(non_finite) => return Ok(non_finite),
                Err(e) => match on_gpu_error(&e) {
                    OnGpuError::CpuForRegion => {}
                    OnGpuError::CpuFromNowOn => gpu = None,
                    OnGpuError::Fail => return Err(e.to_string()),
                },
            }
        }
        slopshop_core::composite::composite_region(document, region, out)
            .map(|report| report.non_finite)
            .map_err(|e| e.to_string())
    }
}

/// What [`export_source`] does after a GPU error.
#[derive(Debug, PartialEq, Eq)]
enum OnGpuError {
    /// Render this region on the CPU; the next one may fit the GPU.
    CpuForRegion,
    /// Render this region and every later one on the CPU: the GPU is short of memory or
    /// failing, and retrying each band would only repeat the failure.
    CpuFromNowOn,
    /// Report the error.
    Fail,
}

fn on_gpu_error(error: &RenderError) -> OnGpuError {
    match error {
        RenderError::TooManyLayers { .. } => OnGpuError::CpuForRegion,
        RenderError::OutOfMemory(_) | RenderError::Gpu(_) => OnGpuError::CpuFromNowOn,
        _ => OnGpuError::Fail,
    }
}

/// GPU buffers of region rendering, kept across the calls of one export so that every band
/// does not allocate (and have wgpu zero) them again. Grown on demand, never beyond the chunk
/// budget.
#[derive(Debug, Default)]
pub(crate) struct RegionBuffers(Option<ChunkBuffers>);

#[derive(Debug)]
struct ChunkBuffers {
    /// Pixels of a chunk, as written by `export_main`.
    output: wgpu::Buffer,
    /// `export_main`'s non-finite counter.
    counter: wgpu::Buffer,
    /// Mappable copy: the chunk's pixels, then the counter right after them.
    readback: wgpu::Buffer,
    /// Pixel bytes the buffers hold.
    capacity: u64,
}

impl RegionBuffers {
    /// Buffers for chunks of up to `bytes` pixel bytes.
    fn reserve(&mut self, device: &wgpu::Device, bytes: u64) -> &ChunkBuffers {
        if self.0.as_ref().is_some_and(|b| b.capacity < bytes) {
            // Freed before the larger ones are allocated.
            self.0 = None;
        }
        self.0.get_or_insert_with(|| ChunkBuffers {
            output: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("region output"),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            counter: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("region non-finite counter"),
                size: COUNTER_BYTES,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            readback: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("region readback"),
                size: bytes + COUNTER_BYTES,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
            capacity: bytes,
        })
    }
}

impl Renderer {
    /// Render `region` of `document` into `out`: premultiplied RGBA f32 in the working space
    /// ([`WORKING_SPACE`](slopshop_core::color::WORKING_SPACE), linear Rec.2020), row-major,
    /// rows of `region.width` pixels. `out` must hold exactly
    /// `region.width × region.height × 4` samples; callers can reuse it across calls.
    ///
    /// Always at full resolution: raster layers are read at pyramid level 0, one texel per
    /// pixel, never at a coarser level. Values are not clipped: finite values are kept as they
    /// are (negative, above 1, above the half-float range). Non-finite values are replaced, like
    /// the CPU reference compositor (`slopshop_core::composite`) does: NaN samples read as 0,
    /// ±inf samples as ±65504 (not ±`f32::MAX`, which the color matrices would spread to the
    /// pixel's other channels), and composited values beyond the f32 range saturate to
    /// ±`f32::MAX`. Returns how many values were replaced (one infinity may count more than
    /// once). Outside a raster smaller than the document, that layer is transparent.
    ///
    /// The tiles needed are uploaded to tile arrays owned by the call: the viewport's cache is
    /// neither used nor evicted. Regions too large for the tile cache or the chunk budget are
    /// split into chunks internally. Blocks until the pixels are read back.
    ///
    /// Fails on an empty region ([`RenderError::EmptyOutput`]), a region not inside the
    /// document, an `out` of the wrong length, when more distinct raster images of one sample
    /// type are visible in the region than the tile cache holds ([`RenderError::TooManyLayers`]),
    /// or when the GPU runs out of memory or reports an error ([`RenderError::OutOfMemory`],
    /// [`RenderError::Gpu`]; never a panic). On error, the content of `out` is unspecified.
    pub fn render_region(
        &self,
        document: &Document,
        region: Rect,
        out: &mut [f32],
    ) -> Result<u64, RenderError> {
        self.render_region_with(document, region, out, &mut RegionBuffers::default())
    }

    /// [`Self::render_region`] with chunk buffers kept by the caller.
    pub(crate) fn render_region_with(
        &self,
        document: &Document,
        region: Rect,
        out: &mut [f32],
        buffers: &mut RegionBuffers,
    ) -> Result<u64, RenderError> {
        if region.is_empty() {
            return Err(RenderError::EmptyOutput);
        }
        let doc = document.size();
        if region.right() > u64::from(doc.width) || region.bottom() > u64::from(doc.height) {
            return Err(RenderError::RegionOutsideDocument {
                region,
                document: doc,
            });
        }
        let expected = region.size().pixel_count() * 4;
        if out.len() as u64 != expected {
            return Err(RenderError::RegionBufferLength {
                expected,
                actual: out.len(),
            });
        }

        // The CPU compositor's steps (ADR 0015): layers at opacity 0 are left out by both, so
        // that both count the same non-finite values.
        document.evaluate_pixels();
        let layers = steps(document);
        let tiles_per_chunk = self.tiles_per_chunk(&layers, region)?;
        let chunks = self.fitted_chunks(
            &layers,
            region_chunks(
                region,
                tiles_per_chunk,
                max_chunk_pixels(self.max_output_bytes),
                self.max_dispatch_pixels,
            ),
        )?;

        // Per-call tile arrays, sized for the chunk that needs the most tiles of each class
        // (tiles of one chunk are never evicted by that chunk: every class holds all its tiles).
        let mut most_tiles = [0usize; 4];
        for (_, needed) in &chunks {
            for (most, &n) in most_tiles.iter_mut().zip(needed) {
                *most = (*most).max(n);
            }
        }
        let chunks: Vec<Rect> = chunks.into_iter().map(|(chunk, _)| chunk).collect();
        let largest = chunks
            .iter()
            .map(|c| c.size().pixel_count())
            .max()
            .unwrap_or(0)
            * PIXEL_BYTES;
        let result = self.capture_errors(|| {
            let mut caches: [Option<TileCache>; 4] = [None, None, None, None];
            for format in GpuTileFormat::ALL {
                let needed = most_tiles[format.index()];
                if needed > 0 {
                    let capacity = self.tile_capacity[format.index()];
                    let needed = u32::try_from(needed).unwrap_or(u32::MAX);
                    caches[format.index()] = Some(TileCache::new(
                        &self.device,
                        format,
                        needed.clamp(1, capacity),
                    ));
                }
            }
            buffers.reserve(&self.device, largest);
            Ok(caches)
        });
        let result = result.and_then(|mut caches| {
            let mut non_finite = 0;
            for chunk in chunks {
                non_finite += self.capture_errors(|| {
                    let buffers = buffers.reserve(&self.device, largest);
                    self.render_chunk(
                        &layers,
                        document.blend_space(),
                        chunk,
                        &mut caches,
                        buffers,
                    )?;
                    self.read_chunk(buffers, chunk, region, out)
                })?;
            }
            Ok(non_finite)
        });
        if result.is_err() {
            // The buffers may be invalid (failed allocation) or still mapped.
            *buffers = RegionBuffers::default();
        }
        result
    }

    /// Most level-0 tiles per chunk such that every class's tiles of a chunk fit in its cache,
    /// counting each image of the region as if it covered the whole chunk.
    fn tiles_per_chunk(&self, layers: &[Step<'_>], region: Rect) -> Result<u64, RenderError> {
        let mut tiles = u64::MAX;
        for format in GpuTileFormat::ALL {
            let images = raster_images(layers, region, format).len();
            if images == 0 {
                continue;
            }
            let capacity = self.tile_capacity[format.index()];
            if images > capacity as usize {
                return Err(RenderError::TooManyLayers { images, capacity });
            }
            tiles = tiles.min(u64::from(capacity) / images as u64);
        }
        Ok(tiles)
    }

    /// `chunks`, each with the tiles it needs per class, split until those fit in the caches:
    /// transformed layers (ADR 0018) can read more tiles than their chunk covers (rotations,
    /// the filter's margin), which `tiles_per_chunk` does not account for.
    fn fitted_chunks(
        &self,
        layers: &[Step<'_>],
        chunks: Vec<Rect>,
    ) -> Result<Vec<(Rect, [usize; 4])>, RenderError> {
        let mut pending = chunks;
        pending.reverse();
        let mut fitted = Vec::new();
        while let Some(chunk) = pending.pop() {
            let needed = chunk_tiles(&chunk_plans(layers, chunk));
            let over = GpuTileFormat::ALL
                .into_iter()
                .find(|f| needed[f.index()] > self.tile_capacity[f.index()] as usize);
            let Some(format) = over else {
                fitted.push((chunk, needed));
                continue;
            };
            // Halve along the longer side; a single pixel that does not fit never will.
            let (a, b) = if chunk.width >= chunk.height && chunk.width > 1 {
                let w = chunk.width / 2;
                (
                    Rect::new(chunk.x, chunk.y, w, chunk.height),
                    Rect::new(chunk.x + w, chunk.y, chunk.width - w, chunk.height),
                )
            } else if chunk.height > 1 {
                let h = chunk.height / 2;
                (
                    Rect::new(chunk.x, chunk.y, chunk.width, h),
                    Rect::new(chunk.x, chunk.y + h, chunk.width, chunk.height - h),
                )
            } else {
                return Err(RenderError::TooManyLayers {
                    images: raster_images(layers, chunk, format).len(),
                    capacity: self.tile_capacity[format.index()],
                });
            };
            pending.push(b);
            pending.push(a);
        }
        Ok(fitted)
    }

    /// Composite one chunk into `buffers.output`, count its non-finite values in
    /// `buffers.counter`, and copy both to `buffers.readback` (pixels from offset 0, the
    /// counter right after them).
    fn render_chunk(
        &self,
        layers: &[Step<'_>],
        blend_space: BlendSpace,
        chunk: Rect,
        caches: &mut [Option<TileCache>; 4],
        buffers: &ChunkBuffers,
    ) -> Result<(), RenderError> {
        let plans = chunk_plans(layers, chunk);
        for cache in caches.iter_mut().flatten() {
            cache.begin_frame();
        }
        let tables: Vec<Vec<u32>> = plans
            .iter()
            .map(|plan| match plan {
                Some(plan) => match caches[plan.format.index()].as_mut() {
                    Some(cache) => self.upload(plan, cache),
                    None => Vec::new(),
                },
                None => Vec::new(),
            })
            .collect();
        // Invariant (`tiles_per_chunk` and the cache sizes): every tile is resident.
        debug_assert!(tables.iter().flatten().all(|&slot| slot != NO_TILE));
        let prepared = encode_layers(layers, &plans, tables, blend_space);

        use wgpu::util::DeviceExt;
        let params = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("region params"),
                contents: &export_params_bytes(chunk, prepared.count),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bytes = chunk.size().pixel_count() * PIXEL_BYTES;
        let layer_buffers = self.layer_buffers(prepared);
        let bind_group = self.bind_group(
            &self.export_bind_group_layout,
            &layer_buffers,
            self.tile_views(caches),
            [
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &buffers.output,
                        offset: 0,
                        size: wgpu::BufferSize::new(bytes),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: buffers.counter.as_entire_binding(),
                },
            ],
        );

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("region"),
            });
        encoder.clear_buffer(&buffers.counter, 0, None);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("region"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.export_pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(
                chunk.width.div_ceil(WORKGROUP_SIZE),
                chunk.height.div_ceil(WORKGROUP_SIZE),
                1,
            );
        }
        encoder.copy_buffer_to_buffer(&buffers.output, 0, &buffers.readback, 0, bytes);
        encoder.copy_buffer_to_buffer(&buffers.counter, 0, &buffers.readback, bytes, COUNTER_BYTES);
        self.queue.submit([encoder.finish()]);

        let (tx, rx) = mpsc::channel();
        buffers.readback.slice(..bytes + COUNTER_BYTES).map_async(
            wgpu::MapMode::Read,
            move |result| {
                // The receiver only disappears if we already returned; nothing to report then.
                let _ = tx.send(result);
            },
        );
        crate::wait_mapped(&self.device, &rx).map_err(RenderError::Readback)
    }

    /// Copy a mapped chunk from `buffers.readback` to its place in `out` (rows of
    /// `region.width` pixels), then unmap; returns the chunk's non-finite count. This copy is
    /// required: mapped GPU memory cannot outlive the mapping, and chunk rows are not
    /// contiguous in `out`.
    fn read_chunk(
        &self,
        buffers: &ChunkBuffers,
        chunk: Rect,
        region: Rect,
        out: &mut [f32],
    ) -> Result<u64, RenderError> {
        let bytes = chunk.size().pixel_count() * PIXEL_BYTES;
        let non_finite;
        {
            let mapped = buffers
                .readback
                .slice(..bytes + COUNTER_BYTES)
                .get_mapped_range()
                .map_err(|e| RenderError::Readback(e.to_string()))?;
            // Chunks lie within the region, whose samples fit in `out`: indices fit in usize.
            let (pixels, counter) = mapped.split_at(bytes as usize);
            let row_samples = chunk.width as usize * 4;
            for (row, src) in pixels.chunks_exact(row_samples * 4).enumerate() {
                let y = (chunk.y - region.y) as usize + row;
                let start = (y * region.width as usize + (chunk.x - region.x) as usize) * 4;
                let dst = &mut out[start..start + row_samples];
                let (samples, _) = src.as_chunks::<4>();
                for (d, s) in dst.iter_mut().zip(samples) {
                    *d = f32::from_ne_bytes(*s);
                }
            }
            let (words, _) = counter.as_chunks::<4>();
            let [low, high] = [0, 1].map(|i| words.get(i).map_or(0, |w| u32::from_ne_bytes(*w)));
            non_finite = u64::from(high) << 32 | u64::from(low);
        }
        buffers.readback.unmap();
        Ok(non_finite)
    }
}

/// Most pixels of one chunk: within the device's binding limit and the chunk budget.
fn max_chunk_pixels(max_output_bytes: u64) -> u64 {
    (max_output_bytes.min(CHUNK_BUDGET_BYTES) / PIXEL_BYTES).max(1)
}

/// Two plans per step, at full resolution, for `chunk`: its raster, then its enabled mask
/// (ADR 0014); `None` where they have nothing in the chunk.
fn chunk_plans<'a>(layers: &[Step<'a>], chunk: Rect) -> Vec<Option<RasterPlan<'a>>> {
    let area = document_area(chunk);
    layers
        .iter()
        .flat_map(|step| {
            step_rasters(step).map(|raster| {
                raster
                    .filter(|&(image, transform)| covers(image, transform, chunk))
                    .and_then(|(image, transform)| {
                        RasterPlan::full_resolution(image, area, transform)
                    })
            })
        })
        .collect()
}

/// Distinct tiles that `plans` read, per storage class.
fn chunk_tiles(plans: &[Option<RasterPlan<'_>>]) -> [usize; 4] {
    let mut keys: [HashSet<TileKey>; 4] = Default::default();
    for plan in plans.iter().flatten() {
        keys[plan.format.index()].extend(plan.keys().flatten());
    }
    keys.map(|k| k.len())
}

fn document_area(area: Rect) -> [f64; 4] {
    [
        f64::from(area.x),
        f64::from(area.y),
        area.right() as f64,
        area.bottom() as f64,
    ]
}

/// Distinct images of a storage class shown by `layers` within `region`.
fn raster_images(layers: &[Step<'_>], region: Rect, format: GpuTileFormat) -> HashSet<ImageId> {
    layers
        .iter()
        .flat_map(|step| step_rasters(step).into_iter().flatten())
        .filter(|&(image, transform)| {
            GpuTileFormat::for_sample(image.stored_format().sample) == format
                && covers(image, transform, region)
        })
        .map(|(image, _)| image.id())
        .collect()
}

/// Whether a raster placed by `transform` has pixels read within `area` (for a resampled one,
/// within the filter's reach).
fn covers(image: &RasterImage, transform: Projective, area: Rect) -> bool {
    let size = image.size();
    if let Some((x, y)) = transform.integer_translation() {
        return x < area.right() as i64
            && y < area.bottom() as i64
            && x + i64::from(size.width) > i64::from(area.x)
            && y + i64::from(size.height) > i64::from(area.y);
    }
    let Some(r) = Resampling::placed(transform, 1.0, image.levels().len(), image.size()) else {
        return false;
    };
    let [x0, y0, x1, y1] = r.source_area(document_area(area));
    x1 > 0.0 && y1 > 0.0 && x0 < f64::from(size.width) && y0 < f64::from(size.height)
}

/// Level-0 tiles an area touches.
#[cfg(test)]
fn tile_span(area: Rect) -> u64 {
    let tile = u64::from(TILE_SIZE);
    let columns = area.right().div_ceil(tile) - u64::from(area.x) / tile;
    let rows = area.bottom().div_ceil(tile) - u64::from(area.y) / tile;
    columns * rows
}

/// Split `region` into chunks rendered by one dispatch each: blocks of whole level-0 tiles
/// (at most `max_tiles` tiles each; full-width bands when a row of tiles fits), each split
/// further into pieces of at most `max_pixels` pixels and `max_side` pixels per side. The
/// chunks partition the region.
fn region_chunks(region: Rect, max_tiles: u64, max_pixels: u64, max_side: u32) -> Vec<Rect> {
    let tile = u64::from(TILE_SIZE);
    let (col0, col1) = (u64::from(region.x) / tile, region.right().div_ceil(tile));
    let (row0, row1) = (u64::from(region.y) / tile, region.bottom().div_ceil(tile));
    let (columns, rows) = (col1 - col0, row1 - row0);
    let max_tiles = max_tiles.max(1);
    let (block_columns, block_rows) = if columns <= max_tiles {
        (columns, (max_tiles / columns).min(rows))
    } else {
        (max_tiles, 1)
    };

    let mut chunks = Vec::new();
    let mut row = row0;
    while row < row1 {
        let y0 = (row * tile).max(u64::from(region.y));
        let y1 = ((row + block_rows) * tile).min(region.bottom());
        let mut column = col0;
        while column < col1 {
            let x0 = (column * tile).max(u64::from(region.x));
            let x1 = ((column + block_columns) * tile).min(region.right());
            split_pixels([x0, y0, x1, y1], max_pixels, max_side, &mut chunks);
            column += block_columns;
        }
        row += block_rows;
    }
    chunks
}

/// Split the area `[x0, y0, x1, y1]` into pieces within the pixel limits.
fn split_pixels(area: [u64; 4], max_pixels: u64, max_side: u32, out: &mut Vec<Rect>) {
    let [x0, y0, x1, y1] = area;
    let max_side = u64::from(max_side.max(1));
    let piece_width = max_side.min(max_pixels.max(1));
    let mut x = x0;
    while x < x1 {
        let width = piece_width.min(x1 - x);
        let piece_height = max_side.min(max_pixels.max(1) / width);
        let mut y = y0;
        while y < y1 {
            let height = piece_height.min(y1 - y);
            // Within the region, whose edges are u32.
            out.push(Rect::new(x as u32, y as u32, width as u32, height as u32));
            y += height;
        }
        x += width;
    }
}

/// Uniform block matching `ExportParams` in composite.wgsl (32 bytes).
fn export_params_bytes(chunk: Rect, layer_count: u32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(32);
    for v in [
        chunk.x,
        chunk.y,
        chunk.width,
        chunk.height,
        layer_count,
        0,
        0,
        0,
    ] {
        bytes.extend(v.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Chunks cover the region exactly once and respect every limit.
    fn check_partition(region: Rect, max_tiles: u64, max_pixels: u64, max_side: u32) -> Vec<Rect> {
        let chunks = region_chunks(region, max_tiles, max_pixels, max_side);
        let covered: u64 = chunks.iter().map(|c| c.size().pixel_count()).sum();
        assert_eq!(covered, region.size().pixel_count(), "{chunks:?}");
        for (i, a) in chunks.iter().enumerate() {
            assert!(!a.is_empty());
            assert_eq!(a.intersection(region), Some(*a), "{a:?} outside {region:?}");
            assert!(a.size().pixel_count() <= max_pixels, "{a:?}");
            assert!(a.width <= max_side && a.height <= max_side, "{a:?}");
            assert!(tile_span(*a) <= max_tiles.max(1), "{a:?}");
            for b in &chunks[i + 1..] {
                assert_eq!(a.intersection(*b), None, "{a:?} overlaps {b:?}");
            }
        }
        chunks
    }

    #[test]
    fn a_region_within_limits_is_one_chunk() {
        let region = Rect::new(10, 20, 1000, 700);
        assert_eq!(check_partition(region, 1024, u64::MAX, 65535), [region]);
        // No raster layer: no tile limit.
        assert_eq!(check_partition(region, u64::MAX, u64::MAX, 65535), [region]);
    }

    #[test]
    fn tile_limits_give_bands_or_column_chunks_on_tile_edges() {
        // 4 × 3 tiles, up to 8 per chunk: bands of two tile rows.
        let region = Rect::new(0, 0, 1024, 768);
        assert_eq!(
            check_partition(region, 8, u64::MAX, 65535),
            [Rect::new(0, 0, 1024, 512), Rect::new(0, 512, 1024, 256)]
        );
        // Unaligned, 3 × 2 tiles touched, 2 per chunk: column chunks cut on tile edges.
        let region = Rect::new(100, 200, 500, 100);
        assert_eq!(
            check_partition(region, 2, u64::MAX, 65535),
            [
                Rect::new(100, 200, 412, 56),
                Rect::new(512, 200, 88, 56),
                Rect::new(100, 256, 412, 44),
                Rect::new(512, 256, 88, 44),
            ]
        );
        check_partition(Rect::new(3, 5, 2000, 1500), 1, u64::MAX, 65535);
    }

    #[test]
    fn buffer_and_dispatch_limits_split_chunks() {
        let region = Rect::new(7, 9, 700, 300);
        // 10 000 pixels per chunk: 14 rows of 700 pixels.
        let chunks = check_partition(region, u64::MAX, 10_000, 65535);
        assert_eq!(chunks[0], Rect::new(7, 9, 700, 14));
        // Narrow dispatches and rows longer than the buffer.
        check_partition(region, u64::MAX, 500, 64);
        check_partition(region, 3, 300, 100);
        check_partition(Rect::new(0, 0, 5, 5), u64::MAX, 1, 1);
    }

    #[test]
    fn chunks_stay_within_the_memory_budget() {
        // DX12 binds up to 2 GiB - 1: the budget, not the binding limit, sizes chunks.
        let dx12 = max_chunk_pixels((1 << 31) - 1);
        assert_eq!(dx12 * PIXEL_BYTES, CHUNK_BUDGET_BYTES);
        // A device with less than the budget keeps its own limit.
        assert_eq!(max_chunk_pixels(64 << 20), (64 << 20) / PIXEL_BYTES);
        assert_eq!(max_chunk_pixels(0), 1);

        // One 256-row band of a 200 000 px wide image (800 MiB of RGBA f32): several chunks,
        // none above the budget.
        let band = Rect::new(0, 256, 200_000, 256);
        let chunks = check_partition(band, 1024, dx12, 65535 * 8);
        assert!(chunks.len() >= 7, "{}", chunks.len());
        for chunk in chunks {
            assert!(chunk.size().pixel_count() * PIXEL_BYTES <= CHUNK_BUDGET_BYTES);
        }
    }

    #[test]
    fn gpu_errors_fall_back_to_the_cpu_when_it_can_help() {
        let too_many = RenderError::TooManyLayers {
            images: 3,
            capacity: 2,
        };
        assert_eq!(on_gpu_error(&too_many), OnGpuError::CpuForRegion);
        for error in [
            RenderError::OutOfMemory("buffer".into()),
            RenderError::Gpu("validation".into()),
        ] {
            assert_eq!(on_gpu_error(&error), OnGpuError::CpuFromNowOn);
        }
        for error in [
            RenderError::Readback("device lost".into()),
            RenderError::EmptyOutput,
        ] {
            assert_eq!(on_gpu_error(&error), OnGpuError::Fail);
        }
    }

    #[test]
    fn tile_spans_count_touched_tiles() {
        assert_eq!(tile_span(Rect::new(0, 0, 256, 256)), 1);
        assert_eq!(tile_span(Rect::new(255, 0, 2, 1)), 2);
        assert_eq!(tile_span(Rect::new(100, 200, 500, 100)), 6);
    }

    #[test]
    fn export_params_match_the_shader_layout() {
        let bytes = export_params_bytes(Rect::new(1, 2, 3, 4), 5);
        assert_eq!(bytes.len(), 32);
        assert_eq!(&bytes[16..20], &5u32.to_le_bytes());
    }
}
