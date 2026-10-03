//! Display cache (ADR 0022): composited tiles of the document's levels, kept on the GPU and
//! addressed by their content.
//!
//! A viewport frame picks the level matching its zoom (as raster layers do), fills the visible
//! tiles of that level that the cache does not hold yet, and presents the view from them
//! (`fill_main` and `present_main` in composite.wgsl). Tiles hold premultiplied linear
//! display-space values (the display transform being linear, filtering them is filtering the
//! composite): out-of-range values clamped to half floats then clip on the display exactly as
//! they would have. A tile's key hashes everything its fill reads: the steps that can change it,
//! encoded for the shader (raster tile slots aside), the images they sample, the display
//! transform, the document size and the tile's place. Edits, undo and redo need no
//! invalidation: they change the keys of the tiles they reach, and only those.

use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;

use slopshop_core::composite::{Step, display_steps};
use slopshop_core::raster::{RasterImage, TILE_SIZE};
use slopshop_core::view::ViewTransform;
use slopshop_core::{BlendSpace, Document, Size};

use crate::tiles::{GpuTileFormat, Slots, TileCache};
use crate::{
    FrameStats, GpuCaches, NO_TILE, PreparedLayers, RasterPlan, Renderer, WORKGROUP_SIZE,
    encode_layers, fit_tile_budget, params_bytes, step_rasters, tile_range, visible_document_rect,
};

/// Cached tiles: premultiplied linear display-space RGBA in half floats.
pub(crate) const CACHE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// VRAM for cached tiles, 512 KiB each: 512 tiles, three 4K viewports at one level.
pub(crate) const CACHE_BUDGET_BYTES: u64 = 256 * 1024 * 1024;
const CACHE_TILE_BYTES: u64 = TILE_SIZE as u64 * TILE_SIZE as u64 * 8;
/// A cache tile is one fill dispatch of `TILE_SIZE²` texels.
const TILE_OUTPUT: Size = Size::new(TILE_SIZE, TILE_SIZE);

/// Tiles the cache holds within [`CACHE_BUDGET_BYTES`] and the device's array layer limit.
pub(crate) fn cache_capacity(max_array_layers: u32) -> u32 {
    let budget = u32::try_from(CACHE_BUDGET_BYTES / CACHE_TILE_BYTES).unwrap_or(u32::MAX);
    budget.min(max_array_layers).max(1)
}

/// What a cached tile holds: a 128-bit hash of everything its fill reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ContentKey([u64; 2]);

/// Two SipHash states (std's) told apart by a first byte: 128 bits, so that two contents giving
/// the same key is not a practical concern for an in-memory cache.
struct KeyHasher([DefaultHasher; 2]);

impl KeyHasher {
    fn new() -> Self {
        let mut hashers = [DefaultHasher::new(), DefaultHasher::new()];
        hashers[1].write_u8(0xa5);
        Self(hashers)
    }

    fn write(&mut self, bytes: &[u8]) {
        for h in &mut self.0 {
            // Length first: concatenations of different parts never collide.
            h.write_usize(bytes.len());
            h.write(bytes);
        }
    }

    fn write_u64(&mut self, v: u64) {
        for h in &mut self.0 {
            h.write_u64(v);
        }
    }

    fn finish(&self) -> ContentKey {
        ContentKey(self.0.each_ref().map(Hasher::finish))
    }
}

/// The cache: one texture array of tiles, slots reassigned least recently used first.
#[derive(Debug)]
pub(crate) struct DisplayCache {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    slots: Slots<ContentKey>,
}

impl DisplayCache {
    pub fn new(device: &wgpu::Device, capacity: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("display cache"),
            size: wgpu::Extent3d {
                width: TILE_SIZE,
                height: TILE_SIZE,
                depth_or_array_layers: capacity,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: CACHE_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("display cache"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        Self {
            texture,
            view,
            slots: Slots::new(capacity),
        }
    }

    /// The layer of `slot`, for a fill to write.
    fn target(&self, slot: u32) -> wgpu::TextureView {
        self.texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("display cache tile"),
            dimension: Some(wgpu::TextureViewDimension::D2),
            base_array_layer: slot,
            array_layer_count: Some(1),
            ..Default::default()
        })
    }
}

/// A canvas area in document pixels: `[x0, y0, x1, y1]`.
type Area = [f64; 4];

fn overlaps(a: Area, b: Area) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

/// Where a raster planned by `plan` can change the document: its placed image, or for a
/// resampled one the points whose filter reads any of its texels.
fn plan_reach(plan: &RasterPlan<'_>) -> Option<Area> {
    let size = plan.image.size();
    match plan.resampling() {
        None => {
            let [x, y] = plan.offset().map(f64::from);
            Some([x, y, x + f64::from(size.width), y + f64::from(size.height)])
        }
        Some(r) => {
            let level = plan.image.levels()[plan.level].size();
            let [eu, ev] = match r.filter {
                slopshop_core::resample::Filter::Nearest => [0.5, 0.5],
                slopshop_core::resample::Filter::Ewa { extent, .. } => extent,
            };
            let texels = [
                -eu,
                -ev,
                f64::from(level.width) + eu,
                f64::from(level.height) + ev,
            ];
            Some(r.to_texel.inverse()?.map_rect(texels))
        }
    }
}

/// Where a step of a fill or raster layer can change the document (its mask restricting it):
/// `None` for one that draws nothing (no plan: nothing of it to sample) and for the other steps.
fn step_reach(
    step: &Step<'_>,
    plan: Option<&RasterPlan<'_>>,
    mask: Option<&RasterPlan<'_>>,
    canvas: Area,
) -> Option<Area> {
    let Step::Layer { layer, .. } = step else {
        return None;
    };
    let mut reach = match &layer.content {
        slopshop_core::LayerContent::Raster { .. } => plan_reach(plan?)?,
        _ => canvas,
    };
    if crate::enabled_mask(layer).is_some() {
        let mask = plan_reach(mask?)?;
        reach = [
            reach[0].max(mask[0]),
            reach[1].max(mask[1]),
            reach[2].min(mask[2]),
            reach[3].min(mask[3]),
        ];
    }
    Some(reach)
}

/// The steps (indices into `steps`) that can change the document `area`: layers reaching it,
/// adjustments of an accumulator that may hold something there (an adjustment leaves
/// transparency as it is), and the groups around them. A group in which nothing is kept changes
/// nothing: left out.
fn steps_reaching(steps: &[Step<'_>], reaches: &[Option<Area>], area: Area) -> Vec<usize> {
    let mut kept = Vec::new();
    // Per open group: where its Begin is in `kept`, and `content` before it.
    let mut groups: Vec<(usize, bool)> = Vec::new();
    // Whether the accumulator may hold something in `area`.
    let mut content = false;
    // The stack steps of the next layer (ADR 0029): kept with it.
    let mut stack: Vec<usize> = Vec::new();
    for (i, step) in steps.iter().enumerate() {
        match step {
            Step::StackOriginal { .. } | Step::StackPaint { .. } | Step::StackEffect { .. } => {
                stack.push(i);
            }
            Step::Layer { .. } => {
                if reaches[i].is_some_and(|reach| overlaps(reach, area)) {
                    kept.append(&mut stack);
                    kept.push(i);
                    content = true;
                }
                stack.clear();
            }
            Step::Adjust { .. } => {
                if content {
                    kept.push(i);
                }
            }
            Step::Begin { isolated } => {
                groups.push((kept.len(), content));
                kept.push(i);
                if *isolated {
                    content = false;
                }
            }
            Step::End { .. } => {
                let Some((begin, outer)) = groups.pop() else {
                    continue;
                };
                if kept.len() == begin + 1 {
                    kept.truncate(begin);
                    content = outer;
                } else {
                    kept.push(i);
                    content |= outer;
                }
            }
        }
    }
    kept
}

/// The level of the document whose texels are not smaller than output pixels of `scale`
/// document pixels (the rule raster layers follow), among `levels`.
fn display_level(scale: f64, levels: usize) -> usize {
    let coarsest = levels.saturating_sub(1);
    if scale > 1.0 {
        (scale.log2().floor() as usize).min(coarsest)
    } else {
        0
    }
}

/// Fill work allowed per progressive frame, in [`Missing::cost`] units: about 8 ms of GPU time
/// on a recent desktop GPU. At least one tile is filled per frame, whatever its cost.
const FILL_BUDGET: u32 = 2048;
/// A resampled raster (or mask) reads tens of texels per pixel: it counts as this many plain ones.
const RESAMPLED_COST: u32 = 16;
/// Where the view's level is not composited yet, a progressive frame shows the finest coarser
/// level whose visible tiles are at most this many (composited at once if needed).
const FALLBACK_TILES: u64 = 4;

/// How a frame uses the display cache.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FrameOptions<'a> {
    /// Timestamp queries (2) for the frame's passes ([`crate::FrameStats::gpu`]).
    pub timestamps: Option<&'a wgpu::QuerySet>,
    /// Composite at most [`FILL_BUDGET`] of missing tiles and show the rest from a coarser
    /// level: the frame reports itself incomplete ([`crate::FrameStats::incomplete`]).
    pub progressive: bool,
}

/// What a frame shows: the document, its steps, the visible area and the view's center.
struct Scene<'a> {
    document: &'a Document,
    steps: Vec<Step<'a>>,
    visible: Option<Area>,
    center: [f64; 2],
}

/// A visible tile that the cache does not hold: what its fill needs.
struct Missing<'a> {
    /// Which level of the frame it belongs to (0: the view's, 1: the coarser one shown where
    /// the view's has no tile yet) and its place in that level's table.
    level: usize,
    index: usize,
    key: ContentKey,
    view: ViewTransform,
    steps: Vec<Step<'a>>,
    plans: Vec<Option<RasterPlan<'a>>>,
    /// Squared distance from its center to the view's, in document pixels: nearest first.
    distance: f64,
}

impl Missing<'_> {
    /// A rough measure of its fill's GPU work: one per step, plus the rasters it samples.
    fn cost(&self) -> u32 {
        let rasters: u32 = self
            .plans
            .iter()
            .flatten()
            .map(|plan| match plan.resampling() {
                Some(_) => RESAMPLED_COST,
                None => 1,
            })
            .sum();
        (self.steps.len() as u32).saturating_add(rasters).max(1)
    }
}

/// The visible tiles of one level of the document: their cache slots, row-major over `range`
/// (`NO_TILE` where missing), and what the missing ones need.
struct LevelTiles<'a> {
    factor: f64,
    range: slopshop_core::Rect,
    table: Vec<u32>,
    missing: Vec<Missing<'a>>,
}

/// The present pass of a frame: `output`-sized RGBA8 view pixels of a `doc`-sized document,
/// written to `output_buffer`, from the cached tiles of one level, and of a coarser one where
/// the first has none.
struct Present<'a> {
    doc: Size,
    view: ViewTransform,
    output: Size,
    output_buffer: &'a wgpu::Buffer,
    levels: Vec<LevelTiles<'a>>,
}

/// A fill ready to be dispatched (its buffers live as long as the bind group needs them).
struct Fill {
    bind_group: wgpu::BindGroup,
}

impl Renderer {
    /// Composite `view` through the display cache into `output_buffer` (as `main` would): record
    /// the fills of the visible tiles the cache lacks and the present pass into the returned
    /// encoder, submitting earlier fills when the raster tile caches run out of slots; the stats
    /// say what was composited and reused (the caller adds the times and uploads). `None`
    /// when the cache cannot show this view (more visible tiles than it holds, or a tile needing
    /// more raster tiles than their caches hold): the caller composites directly. Tiles filled
    /// before that stay cached.
    pub(crate) fn composite_cached(
        &self,
        document: &Document,
        view: ViewTransform,
        output: Size,
        output_buffer: &wgpu::Buffer,
        options: FrameOptions<'_>,
        caches: &mut GpuCaches,
    ) -> Option<(wgpu::CommandEncoder, FrameStats)> {
        let mut stats = FrameStats::default();
        let doc = document.size();
        let levels = RasterImage::level_sizes(doc).len();
        let level = display_level(view.scale, levels);
        let scene = Scene {
            document,
            // Stacks not evaluated yet are evaluated by the shader (ADR 0029).
            steps: display_steps(document),
            visible: visible_document_rect(doc, view, output),
            center: view.output_to_document(
                f64::from(output.width) / 2.0,
                f64::from(output.height) / 2.0,
            ),
        };
        let capacity = self.display_capacity;
        let display = caches
            .display
            .get_or_insert_with(|| DisplayCache::new(&self.device, capacity));
        display.slots.begin_frame();

        let mut fine = self.level_tiles(&scene, level, display, &mut stats)?;
        let mut to_fill = Vec::new();
        let mut coarse = None;
        if options.progressive && !fine.missing.is_empty() {
            // Nearest the view's center first, within the budget.
            fine.missing
                .sort_by(|a, b| a.distance.total_cmp(&b.distance));
            let mut spent = 0u32;
            let mut deferred = false;
            fine.missing.retain(|tile| {
                let cost = tile.cost();
                let fits = spent == 0 || spent.saturating_add(cost) <= FILL_BUDGET;
                if fits {
                    spent = spent.saturating_add(cost);
                } else {
                    deferred = true;
                }
                fits
            });
            if deferred {
                stats.incomplete = true;
                // The finest coarser level with few visible tiles, shown where the view's
                // level has none yet; its own missing tiles are composited now (once: they stay
                // cached while the view moves within them).
                let fallback = (level + 1..levels)
                    .find(|&l| self.visible_tiles(&scene, l) <= FALLBACK_TILES)
                    .unwrap_or(levels - 1);
                if fallback > level {
                    let mut tiles = self.level_tiles(&scene, fallback, display, &mut stats)?;
                    for tile in &mut tiles.missing {
                        tile.level = 1;
                    }
                    to_fill.append(&mut tiles.missing);
                    coarse = Some(tiles);
                }
            }
        }
        to_fill.append(&mut fine.missing);
        let mut levels_shown = vec![fine];
        levels_shown.extend(coarse);

        // Fill the missing tiles, all of this frame's slots being protected from reassignment.
        let mut encoder = self.encoder();
        let mut fills = Vec::new();
        let mut timing = options.timestamps;
        if !to_fill.is_empty() {
            for format in GpuTileFormat::ALL {
                let needed = to_fill
                    .iter()
                    .flat_map(|m| m.plans.iter().flatten())
                    .any(|p| p.format == format);
                if needed {
                    let capacity = self.tile_capacity[format.index()];
                    caches.tiles[format.index()]
                        .get_or_insert_with(|| TileCache::new(&self.device, format, capacity))
                        .begin_frame();
                }
            }
        }
        for tile in to_fill {
            let mut slots = self.upload_all(&tile.plans, &mut caches.tiles);
            if slots.is_none() && !fills.is_empty() {
                // The raster caches are full of this frame's tiles: run the fills recorded so
                // far, after which their tiles can be reassigned.
                self.record_fills(&mut encoder, &fills, timing.take(), false);
                self.queue
                    .submit([std::mem::replace(&mut encoder, self.encoder()).finish()]);
                fills.clear();
                for cache in caches.tiles.iter_mut().flatten() {
                    cache.begin_frame();
                }
                slots = self.upload_all(&tile.plans, &mut caches.tiles);
            }
            let display = caches.display.as_mut()?;
            let (Some(slots), Some(slot)) = (slots, display.slots.insert(tile.key)) else {
                display.slots.remove(&tile.key);
                if !fills.is_empty() {
                    self.record_fills(&mut encoder, &fills, timing.take(), false);
                    self.queue.submit([encoder.finish()]);
                }
                return None;
            };
            let prepared = encode_layers(&tile.steps, &tile.plans, slots, document.blend_space());
            stats.layers = stats.layers.max(prepared.count);
            stats.tiles_composited += 1;
            fills.push(self.fill(
                document,
                tile.view,
                prepared,
                display.target(slot),
                &caches.tiles,
            ));
            levels_shown[tile.level].table[tile.index] = slot;
        }
        if !fills.is_empty() {
            self.record_fills(&mut encoder, &fills, timing.take(), false);
        }
        drop(fills);

        // Present: the view from the cached tiles.
        let display = caches.display.as_ref()?;
        let present = Present {
            doc,
            view,
            output,
            output_buffer,
            levels: levels_shown,
        };
        self.record_present(
            &mut encoder,
            present,
            display,
            options.timestamps.map(|q| (q, timing.is_some())),
        );
        Some((encoder, stats))
    }

    /// How many tiles of `level` the scene's visible area covers.
    fn visible_tiles(&self, scene: &Scene<'_>, level: usize) -> u64 {
        let range = level_range(scene, level);
        u64::from(range.width) * u64::from(range.height)
    }

    /// The visible tiles of `level`: those `display` holds (now used by this frame), and what
    /// the others need. `None` when they are more than the cache holds.
    fn level_tiles<'a>(
        &self,
        scene: &Scene<'a>,
        level: usize,
        display: &mut DisplayCache,
        stats: &mut FrameStats,
    ) -> Option<LevelTiles<'a>> {
        let doc = scene.document.size();
        let canvas = [0.0, 0.0, f64::from(doc.width), f64::from(doc.height)];
        let factor = f64::from(1u32 << level);
        let tile_span = f64::from(TILE_SIZE) * factor;
        let range = level_range(scene, level);
        if u64::from(range.width) * u64::from(range.height) > u64::from(self.display_capacity) {
            return None;
        }
        let steps = &scene.steps;

        // Every step planned at this level over the visible area, with where it can reach.
        let plans: Vec<Option<RasterPlan<'a>>> = steps
            .iter()
            .flat_map(|step| {
                step_rasters(step).map(|raster| {
                    let (image, transform) = raster?;
                    RasterPlan::new(image, scene.visible?, transform, factor)
                })
            })
            .collect();
        let reaches: Vec<Option<Area>> = steps
            .iter()
            .enumerate()
            .map(|(i, step)| {
                step_reach(
                    step,
                    plans[2 * i].as_ref(),
                    plans[2 * i + 1].as_ref(),
                    canvas,
                )
            })
            .collect();
        // What each step contributes to the keys, once per frame.
        let step_keys: Vec<ContentKey> = steps
            .iter()
            .enumerate()
            .map(|(i, step)| {
                step_key(
                    step,
                    [plans[2 * i], plans[2 * i + 1]],
                    scene.document.blend_space(),
                )
            })
            .collect();

        // Keys of the visible tiles: cached ones are used as they are.
        let mut table = Vec::with_capacity((range.width * range.height) as usize);
        let mut missing = Vec::new();
        for row in range.y..range.y + range.height {
            for col in range.x..range.x + range.width {
                let corner = [f64::from(col) * tile_span, f64::from(row) * tile_span];
                let area = [
                    corner[0],
                    corner[1],
                    (corner[0] + tile_span).min(canvas[2]),
                    (corner[1] + tile_span).min(canvas[3]),
                ];
                let kept = steps_reaching(steps, &reaches, area);
                let key = tile_key(
                    doc,
                    [level as u32, col, row],
                    self.tile_capacity,
                    kept.iter().map(|&i| step_keys[i]),
                );
                if let Some(slot) = display.slots.get(&key) {
                    table.push(slot);
                    stats.tiles_reused += 1;
                    continue;
                }
                let tile_steps = kept.iter().map(|&i| steps[i]).collect();
                let mut tile_plans: Vec<Option<RasterPlan<'a>>> = kept
                    .iter()
                    .flat_map(|&i| {
                        [2 * i, 2 * i + 1].map(|p| {
                            plans[p]
                                .map(|plan| RasterPlan { area, ..plan })
                                .filter(|plan| !plan.range().is_empty())
                        })
                    })
                    .collect();
                // As a direct frame would: coarser levels rather than missing layers when the
                // tile reads more raster tiles than their caches hold. A function of what the
                // key holds (plans and capacities), so the key needs no more.
                fit_tile_budget(&mut tile_plans, self.tile_capacity);
                let middle = [
                    (area[0] + area[2]) / 2.0 - scene.center[0],
                    (area[1] + area[3]) / 2.0 - scene.center[1],
                ];
                missing.push(Missing {
                    level: 0,
                    index: table.len(),
                    key,
                    view: ViewTransform {
                        origin: corner,
                        scale: factor,
                    },
                    steps: tile_steps,
                    plans: tile_plans,
                    distance: middle[0] * middle[0] + middle[1] * middle[1],
                });
                table.push(NO_TILE);
            }
        }
        Some(LevelTiles {
            factor,
            range,
            table,
            missing,
        })
    }

    fn encoder(&self) -> wgpu::CommandEncoder {
        self.device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("display cache"),
            })
    }

    /// Make every planned raster tile resident (`None` when a cache has no slot left for this
    /// frame), two plans per step as `encode_layers` takes them.
    fn upload_all(
        &self,
        plans: &[Option<RasterPlan<'_>>],
        tiles: &mut [Option<TileCache>; 4],
    ) -> Option<Vec<Vec<u32>>> {
        plans
            .iter()
            .map(|plan| match plan {
                Some(plan) => self.try_upload(plan, tiles[plan.format.index()].as_mut()?),
                None => Some(Vec::new()),
            })
            .collect()
    }

    /// The bind group of one fill: `prepared` composited for the tile `view` into `target`.
    fn fill(
        &self,
        document: &Document,
        view: ViewTransform,
        prepared: PreparedLayers,
        target: wgpu::TextureView,
        tiles: &[Option<TileCache>; 4],
    ) -> Fill {
        use wgpu::util::DeviceExt;
        let params = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fill params"),
                contents: &params_bytes(document.size(), view, TILE_OUTPUT, prepared.count),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let buffers = self.layer_buffers(prepared);
        let bind_group = self.bind_group(
            &self.fill_bind_group_layout,
            &buffers,
            self.tile_views(tiles),
            [
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 12,
                    resource: wgpu::BindingResource::TextureView(&target),
                },
            ],
        );
        Fill { bind_group }
    }

    /// One compute pass of `fills`, writing the frame's first timestamp when given (and its last
    /// with `end_timestamp`).
    fn record_fills(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        fills: &[Fill],
        begin_timestamp: Option<&wgpu::QuerySet>,
        end_timestamp: bool,
    ) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("display cache fill"),
            timestamp_writes: begin_timestamp.map(|query_set| wgpu::ComputePassTimestampWrites {
                query_set,
                beginning_of_pass_write_index: Some(0),
                end_of_pass_write_index: end_timestamp.then_some(1),
            }),
        });
        pass.set_pipeline(&self.fill_pipeline);
        let groups = TILE_SIZE.div_ceil(WORKGROUP_SIZE);
        for fill in fills {
            pass.set_bind_group(0, &fill.bind_group, &[]);
            pass.dispatch_workgroups(groups, groups, 1);
        }
    }

    /// The present pass of `present` from `display`. With `timestamps`, the pass writes the
    /// frame's last timestamp, and its first when asked (no fill wrote it).
    fn record_present(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        present: Present<'_>,
        display: &DisplayCache,
        timestamps: Option<(&wgpu::QuerySet, bool)>,
    ) {
        use wgpu::util::DeviceExt;
        let Present {
            doc,
            view,
            output,
            output_buffer,
            levels,
        } = present;
        let params = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("present params"),
                contents: &params_bytes(doc, view, output, 0),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let cache_params = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("present cache params"),
                contents: &cache_params_bytes(&levels),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        // The levels' tables one after the other; a binding cannot be empty.
        let mut table: Vec<u32> = levels
            .iter()
            .flat_map(|l| l.table.iter().copied())
            .collect();
        if table.is_empty() {
            table.push(NO_TILE);
        }
        let table_bytes: Vec<u8> = table.iter().flat_map(|v| v.to_le_bytes()).collect();
        let table = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("present cache table"),
                contents: &table_bytes,
                usage: wgpu::BufferUsages::STORAGE,
            });
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("present"),
            layout: &self.present_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: output_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 13,
                    resource: cache_params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 14,
                    resource: wgpu::BindingResource::TextureView(&display.view),
                },
                wgpu::BindGroupEntry {
                    binding: 15,
                    resource: table.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("display cache present"),
            timestamp_writes: timestamps.map(|(query_set, begins)| {
                wgpu::ComputePassTimestampWrites {
                    query_set,
                    beginning_of_pass_write_index: begins.then_some(0),
                    end_of_pass_write_index: Some(1),
                }
            }),
        });
        pass.set_pipeline(&self.present_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(
            output.width.div_ceil(WORKGROUP_SIZE),
            output.height.div_ceil(WORKGROUP_SIZE),
            1,
        );
    }
}

/// What `step` contributes to the key of a tile it reaches: its fields as the shader reads them,
/// encoded from its `plans` (raster, mask) placed nowhere (so that nothing depends on the tile's
/// area or on cache slots: the tile's place is in its key), the curves' lookup tables, and the
/// images sampled with the levels planned (immutable images: an id is their content).
fn step_key(
    step: &Step<'_>,
    plans: [Option<RasterPlan<'_>>; 2],
    blend_space: BlendSpace,
) -> ContentKey {
    let nowhere = plans.map(|plan| {
        plan.map(|plan| RasterPlan {
            area: [0.0; 4],
            ..plan
        })
    });
    let encoded = encode_layers(
        std::slice::from_ref(step),
        &nowhere,
        Vec::new(),
        blend_space,
    );
    let mut hasher = KeyHasher::new();
    hasher.write_u64(u64::from(encoded.count));
    hasher.write(&encoded.bytes);
    // Without slots, the table holds only curves' lookup tables.
    let table: Vec<u8> = encoded
        .tile_table
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    hasher.write(&table);
    for plan in plans.iter().flatten() {
        hasher.write_u64(plan.image.id().get());
        hasher.write_u64(plan.level as u64);
    }
    hasher.finish()
}

/// The key of tile `[level, col, row]` of a `doc`-sized document composited from the steps of
/// `step_keys`, with raster tile caches of `capacity` (which decides the levels when a tile
/// reads more than they hold). The display transform is in it too.
fn tile_key(
    doc: Size,
    place: [u32; 3],
    capacity: [u32; 4],
    step_keys: impl Iterator<Item = ContentKey>,
) -> ContentKey {
    let mut hasher = KeyHasher::new();
    for v in [doc.width, doc.height]
        .iter()
        .chain(&place)
        .chain(&capacity)
    {
        hasher.write_u64(u64::from(*v));
    }
    let display: Vec<u8> = crate::display_matrix()
        .iter()
        .flatten()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    hasher.write(&display);
    let mut steps = 0u64;
    for ContentKey([a, b]) in step_keys {
        hasher.write_u64(a);
        hasher.write_u64(b);
        steps += 1;
    }
    hasher.write_u64(steps);
    hasher.finish()
}

/// The visible tiles of `level` for `scene`.
fn level_range(scene: &Scene<'_>, level: usize) -> slopshop_core::Rect {
    let size = RasterImage::level_sizes(scene.document.size())[level];
    match scene.visible {
        Some(visible) => tile_range(
            visible,
            f64::from(1u32 << level),
            size.width.div_ceil(TILE_SIZE),
            size.height.div_ceil(TILE_SIZE),
        ),
        None => slopshop_core::Rect::new(0, 0, 0, 0),
    }
}

/// Uniform block matching `CacheParams` in composite.wgsl (80 bytes): the levels shown (the
/// second one where the first has no tile), their tables one after the other.
fn cache_params_bytes(levels: &[LevelTiles<'_>]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(80);
    let mut offset = 0u32;
    for level in levels.iter().take(2) {
        let r = level.range;
        for v in [r.x, r.y, r.width, r.height] {
            bytes.extend(v.to_le_bytes());
        }
        bytes.extend((level.factor as f32).to_le_bytes());
        bytes.extend(offset.to_le_bytes());
        bytes.resize(bytes.len() + 8, 0);
        offset += level.table.len() as u32;
    }
    bytes.resize(64, 0);
    let fallback = u32::from(levels.len() > 1);
    bytes.extend(fallback.to_le_bytes());
    bytes.resize(80, 0);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_levels_follow_the_zoom() {
        assert_eq!(display_level(0.25, 6), 0);
        assert_eq!(display_level(1.0, 6), 0);
        assert_eq!(display_level(1.9, 6), 0);
        assert_eq!(display_level(2.0, 6), 1);
        assert_eq!(display_level(5.0, 6), 2);
        assert_eq!(display_level(1000.0, 6), 5);
    }

    #[test]
    fn keys_tell_contents_apart() {
        let key = |parts: &[&[u8]]| {
            let mut h = KeyHasher::new();
            for part in parts {
                h.write(part);
            }
            h.finish()
        };
        assert_eq!(key(&[b"ab", b"c"]), key(&[b"ab", b"c"]));
        assert_ne!(key(&[b"ab", b"c"]), key(&[b"a", b"bc"]));
        let ContentKey([a, b]) = key(&[b"x"]);
        assert_ne!(a, b);
    }
}
