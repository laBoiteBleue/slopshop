//! Display cache (ADR 0022): composited tiles of the document's levels, kept on the GPU and
//! addressed by their content.
//!
//! A viewport frame picks the level matching its zoom (as raster layers do), fills the visible
//! tiles of that level that the cache does not hold yet, and presents the view from them
//! (`fill_main` and `present_main` in composite.wgsl). Tiles hold premultiplied linear
//! display-space values (the display transform being linear, filtering them is filtering the
//! composite): out-of-range values clamped to half floats then clip on the display exactly as
//! they would have. A tile's key hashes everything its fill reads: the steps that can change it,
//! encoded for the shader (raster tile slots aside), the raster tiles they sample (by identity:
//! images are immutable and share the tiles they do not change, so an image changed elsewhere
//! leaves the key of a tile away from the change as it was), the display transform, the
//! document size and the tile's place. Edits, undo and redo need no invalidation: they change
//! the keys of the tiles they reach, and only those.

use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;
use std::sync::Arc;

use slopshop_core::composite::{Step, display_steps_with};
use slopshop_core::raster::{RasterImage, TILE_SIZE};
use slopshop_core::stack::Looks;
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

/// A fast 128-bit fold of many words (two multiplicative lanes), for what is hashed per tile
/// and per raster tile it reads: hundreds of layers over hundreds of tiles every frame, where
/// SipHash per word would cost more than the rest of the frame's planning. The words are
/// addresses and keys, never chosen by anyone; the fold is fed to the [`KeyHasher`] at the end.
struct Fold([u64; 2]);

impl Fold {
    fn new() -> Self {
        Self([0x243f_6a88_85a3_08d3, 0x1319_8a2e_0370_7344])
    }

    fn add(&mut self, word: u64) {
        let [a, b] = &mut self.0;
        *a = (a.rotate_left(5) ^ word).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        *b = (b.rotate_left(23) ^ word).wrapping_mul(0xc2b2_ae3d_27d4_eb4f);
    }

    fn write_to(&self, hasher: &mut KeyHasher) {
        hasher.write_u64(self.0[0]);
        hasher.write_u64(self.0[1]);
    }
}

/// Two SipHash states (std's) told apart by a first byte: 128 bits, so that two contents giving
/// the same key is not a practical concern for an in-memory cache.
#[derive(Clone)]
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
    /// The whole texture array (it keeps the texture): read by the present pass, written by the
    /// fills (a tile a layer).
    view: wgpu::TextureView,
    slots: Slots<ContentKey>,
    /// Per slot, the raster tiles its key was made from. A key holds their addresses: while the
    /// tile is cached these stay allocated, so no other tile can take an address a cached key
    /// holds (a stroke's frames drop their tiles). Replaced when the slot is reassigned.
    pins: Vec<Vec<Arc<[u8]>>>,
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
            view,
            slots: Slots::new(capacity),
            pins: vec![Vec::new(); capacity as usize],
        }
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
    /// Keep the document's alpha, without the checkerboard (thumbnails).
    pub transparent: bool,
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
    /// The raster tiles its key was made from ([`DisplayCache::pins`]).
    pins: Vec<Arc<[u8]>>,
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
    transparent: bool,
}

/// A fill ready to be dispatched (its buffers live as long as the bind group needs them).
/// The fills of a frame recorded so far, run in one dispatch (`fill_main`): their layers and tile
/// tables one after the other, and each tile's view, slot and where its own start.
#[derive(Default)]
struct Fills {
    layers: Vec<u8>,
    layer_count: u32,
    table: Vec<u32>,
    /// One `Fill` of composite.wgsl a tile.
    headers: Vec<u8>,
    count: u32,
}

impl Fills {
    fn push(&mut self, view: ViewTransform, prepared: PreparedLayers, slot: u32) {
        for v in [
            view.origin[0] as f32,
            view.origin[1] as f32,
            view.scale as f32,
        ] {
            self.headers.extend(v.to_le_bytes());
        }
        let table_base = self.table.len() as u32;
        for v in [prepared.count, self.layer_count, table_base, slot, 0] {
            self.headers.extend(v.to_le_bytes());
        }
        self.layer_count += prepared.count;
        self.layers.extend(prepared.bytes);
        self.table.extend(prepared.tile_table);
        self.count += 1;
    }

    fn is_empty(&self) -> bool {
        self.count == 0
    }
}

impl Renderer {
    /// Composite `view` through the display cache into `output_buffer` (as `main` would): record
    /// the fills of the visible tiles the cache lacks and the present pass into the returned
    /// encoder, submitting earlier fills when the raster tile caches run out of slots; the stats
    /// say what was composited and reused (the caller adds the times and uploads). `None`
    /// when the cache cannot show this view (more visible tiles than it holds, or a tile needing
    /// more raster tiles than their caches hold): the caller composites directly. Tiles filled
    /// before that stay cached.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn composite_cached(
        &self,
        document: &Document,
        looks: &Looks,
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
            // Stacks not evaluated yet are evaluated by the shader (ADR 0029), those with a
            // filter over the look at what is shown (ADR 0034).
            steps: display_steps_with(document, Some(looks)),
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
        // What the fills write, the whole texture array (a tile a layer).
        let target = display.view.clone();

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
        let mut fills = Fills::default();
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
                self.record_fills(
                    &mut encoder,
                    document,
                    std::mem::take(&mut fills),
                    &target,
                    &caches.tiles,
                    timing.take(),
                );
                self.queue
                    .submit([std::mem::replace(&mut encoder, self.encoder()).finish()]);
                for cache in caches.tiles.iter_mut().flatten() {
                    cache.begin_frame();
                }
                slots = self.upload_all(&tile.plans, &mut caches.tiles);
            }
            let display = caches.display.as_mut()?;
            let (Some(slots), Some(slot)) = (slots, display.slots.insert(tile.key)) else {
                display.slots.remove(&tile.key);
                if !fills.is_empty() {
                    self.record_fills(
                        &mut encoder,
                        document,
                        fills,
                        &target,
                        &caches.tiles,
                        timing.take(),
                    );
                    self.queue.submit([encoder.finish()]);
                }
                return None;
            };
            // The slot's old key is gone (reassigned): so can its tiles be.
            if let Some(pins) = display.pins.get_mut(slot as usize) {
                *pins = tile.pins;
            }
            let prepared = encode_layers(&tile.steps, &tile.plans, slots, document.blend_space());
            stats.layers = stats.layers.max(prepared.count);
            stats.tiles_composited += 1;
            fills.push(tile.view, prepared, slot);
            levels_shown[tile.level].table[tile.index] = slot;
        }
        if !fills.is_empty() {
            self.record_fills(
                &mut encoder,
                document,
                fills,
                &target,
                &caches.tiles,
                timing.take(),
            );
        }

        // Present: the view from the cached tiles.
        let display = caches.display.as_ref()?;
        let present = Present {
            doc,
            view,
            output,
            output_buffer,
            levels: levels_shown,
            transparent: options.transparent,
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
        let factor = f64::from(1u32 << level);
        let range = level_range(scene, level);
        if u64::from(range.width) * u64::from(range.height) > u64::from(self.display_capacity) {
            return None;
        }
        let mut planned = LevelPlan::new(scene, level, self.tile_capacity);

        // Keys of the visible tiles: cached ones are used as they are.
        let mut table = Vec::with_capacity((range.width * range.height) as usize);
        let mut missing = Vec::new();
        for row in range.y..range.y + range.height {
            for col in range.x..range.x + range.width {
                let tile = planned.keyed(col, row);
                if let Some(slot) = display.slots.get(&tile.key) {
                    table.push(slot);
                    stats.tiles_reused += 1;
                    continue;
                }
                let (area, key) = (tile.area, tile.key);
                let tile_steps = tile.kept.iter().map(|&i| scene.steps[i]).collect();
                let tile_plans = planned.plans_of(tile);
                let pins = tile_plans
                    .iter()
                    .flatten()
                    .flat_map(|plan| plan.tile_refs(plan.range()).flatten().map(Arc::clone))
                    .collect();
                let middle = [
                    (area[0] + area[2]) / 2.0 - scene.center[0],
                    (area[1] + area[3]) / 2.0 - scene.center[1],
                ];
                missing.push(Missing {
                    level: 0,
                    index: table.len(),
                    key,
                    view: ViewTransform {
                        origin: [
                            f64::from(col) * f64::from(TILE_SIZE) * factor,
                            f64::from(row) * f64::from(TILE_SIZE) * factor,
                        ],
                        scale: factor,
                    },
                    steps: tile_steps,
                    plans: tile_plans,
                    pins,
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

    /// One dispatch of `fills` into the display cache, writing the frame's first timestamp when
    /// given.
    fn record_fills(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        document: &Document,
        fills: Fills,
        display: &wgpu::TextureView,
        tiles: &[Option<TileCache>; 4],
        begin_timestamp: Option<&wgpu::QuerySet>,
    ) {
        use wgpu::util::DeviceExt;
        let buffer = |label, contents: &[u8], usage| {
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents,
                    usage,
                })
        };
        // The views are the fills' own: the frame's only gives the document and the display.
        let frame = ViewTransform {
            origin: [0.0, 0.0],
            scale: 1.0,
        };
        let params = buffer(
            "fill params",
            &params_bytes(document.size(), frame, TILE_OUTPUT, 0, false),
            wgpu::BufferUsages::UNIFORM,
        );
        let buffers = self.layer_buffers(PreparedLayers {
            count: fills.layer_count,
            bytes: fills.layers,
            tile_table: fills.table,
        });
        let headers = buffer("fills", &fills.headers, wgpu::BufferUsages::STORAGE);
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
                    resource: wgpu::BindingResource::TextureView(display),
                },
                wgpu::BindGroupEntry {
                    binding: 16,
                    resource: headers.as_entire_binding(),
                },
            ],
        );
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("display cache fill"),
            timestamp_writes: begin_timestamp.map(|query_set| wgpu::ComputePassTimestampWrites {
                query_set,
                beginning_of_pass_write_index: Some(0),
                end_of_pass_write_index: None,
            }),
        });
        pass.set_pipeline(&self.fill_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        let groups = TILE_SIZE.div_ceil(WORKGROUP_SIZE);
        pass.dispatch_workgroups(groups, groups, fills.count);
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
            transparent,
        } = present;
        let params = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("present params"),
                contents: &params_bytes(doc, view, output, 0, transparent),
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
/// area or on cache slots: the tile's place is in its key), the curves' lookup tables, and what
/// the images sampled are but for their tiles (those are in the tile's key, see [`tile_key`]).
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
    // The planned level, and what decides how far a tile budget may coarsen it and how the
    // tiles' bytes read: the image's size and storage. (The fields hold the rest.)
    for plan in plans.iter().flatten() {
        let size = plan.image.size();
        let stored = plan.image.stored_format();
        hasher.write_u64(plan.level as u64);
        hasher.write_u64(u64::from(size.width) << 32 | u64::from(size.height));
        hasher.write_u64(
            stored.layout as u64 | (stored.sample as u64) << 8 | (stored.alpha as u64) << 16,
        );
    }
    hasher.finish()
}

/// The plans of the `kept` steps of the tile over `area` (two per step, from the frame's
/// `plans`), those with nothing to sample there left out, coarsened as a direct frame's would be
/// when they read more raster tiles than the caches of `capacity` hold.
fn fitted_plans<'a>(
    kept: &[usize],
    plans: &[Option<RasterPlan<'a>>],
    area: Area,
    capacity: [u32; 4],
) -> Vec<Option<RasterPlan<'a>>> {
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
    fit_tile_budget(&mut tile_plans, capacity);
    tile_plans
}

/// What every tile of a frame's keys starts from: the document's size, the raster tile caches'
/// `capacity` (which decides the levels when a tile reads more than they hold) and the display
/// transform.
fn tile_seed(doc: Size, capacity: [u32; 4]) -> KeyHasher {
    let mut hasher = KeyHasher::new();
    for v in [doc.width, doc.height].iter().chain(&capacity) {
        hasher.write_u64(u64::from(*v));
    }
    let display: Vec<u8> = crate::display_matrix()
        .iter()
        .flatten()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    hasher.write(&display);
    hasher
}

/// The key of tile `place` (`[level, col, row]`) composited from the steps of `step_keys`, from
/// the `seed` of its frame ([`tile_seed`]).
/// `plans` are the steps' rasters, two per step (raster, mask), each with the tiles of its level
/// that the fill reads (`None`: nothing to sample), in the order of the steps.
///
/// What a fill reads of a raster is its tiles: their identities are in the key (the address of
/// each tile's allocation: tiles are immutable and an image changed in one place shares all its
/// other tiles, so a tile away from the change keeps its key, ADR 0027). An address stands for
/// its tile because a cached tile keeps its tiles allocated ([`DisplayCache::pins`]); a tile
/// that is not stored is a zero. The placement is in the step's key and the tile's place; the
/// level read, in the plan, is in here.
fn tile_key<'p, 'a: 'p>(
    seed: &KeyHasher,
    place: [u32; 3],
    step_keys: impl Iterator<Item = ContentKey>,
    plans: impl Iterator<Item = Option<(&'p RasterPlan<'a>, slopshop_core::Rect)>>,
) -> ContentKey {
    let mut hasher = seed.clone();
    for v in place {
        hasher.write_u64(u64::from(v));
    }
    let mut fold = Fold::new();
    let mut steps = 0u64;
    for ContentKey([a, b]) in step_keys {
        fold.add(a);
        fold.add(b);
        steps += 1;
    }
    fold.add(steps);
    for plan in plans {
        let Some((plan, range)) = plan else {
            fold.add(0);
            continue;
        };
        // The level and how many tiles follow (never 0: a plan reads at least one).
        let count = u64::from(range.width) * u64::from(range.height);
        fold.add((plan.level as u64 + 1) | count << 8);
        plan.for_each_tile(range, |tile| {
            fold.add(tile.map_or(0, |tile| tile.as_ptr() as usize as u64));
        });
    }
    fold.write_to(&mut hasher);
    hasher.finish()
}

/// The steps of a frame planned at one level of the document: where each can reach, what each
/// adds to a key, and from them the key of any tile of the level.
struct LevelPlan<'a, 's> {
    steps: &'s [Step<'a>],
    level: usize,
    /// The canvas, in document pixels.
    canvas: Area,
    /// Every step planned over the visible area (two per step: its raster, its mask).
    plans: Vec<Option<RasterPlan<'a>>>,
    /// Where each step can reach.
    reaches: Vec<Option<Area>>,
    /// What each step contributes to the keys, once per frame.
    step_keys: Vec<ContentKey>,
    seed: KeyHasher,
    capacity: [u32; 4],
    /// Scratch: the raster tiles each plan of the tile at hand reads.
    ranges: Vec<Option<slopshop_core::Rect>>,
}

/// A tile of a level with its key.
struct KeyedTile<'a> {
    /// The document area it covers.
    area: Area,
    /// The steps (indices) that reach it.
    kept: Vec<usize>,
    key: ContentKey,
    /// Its plans, when they were needed to make the key.
    fitted: Option<Vec<Option<RasterPlan<'a>>>>,
}

impl<'a, 's> LevelPlan<'a, 's> {
    fn new(scene: &'s Scene<'a>, level: usize, capacity: [u32; 4]) -> Self {
        let doc = scene.document.size();
        let canvas = [0.0, 0.0, f64::from(doc.width), f64::from(doc.height)];
        let factor = f64::from(1u32 << level);
        let steps = &scene.steps[..];
        let plans: Vec<Option<RasterPlan<'a>>> = steps
            .iter()
            .flat_map(|step| {
                step_rasters(step).map(|raster| {
                    let (image, transform) = raster?;
                    RasterPlan::new(image, scene.visible?, transform, factor)
                })
            })
            .collect();
        let reaches = steps
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
        let step_keys = steps
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
        Self {
            steps,
            level,
            canvas,
            plans,
            reaches,
            step_keys,
            seed: tile_seed(doc, capacity),
            capacity,
            ranges: Vec::new(),
        }
    }

    /// Tile (`col`, `row`) of the level, with its key.
    fn keyed(&mut self, col: u32, row: u32) -> KeyedTile<'a> {
        let span = f64::from(TILE_SIZE) * f64::from(1u32 << self.level);
        let corner = [f64::from(col) * span, f64::from(row) * span];
        let area = [
            corner[0],
            corner[1],
            (corner[0] + span).min(self.canvas[2]),
            (corner[1] + span).min(self.canvas[3]),
        ];
        let kept = steps_reaching(self.steps, &self.reaches, area);
        // The raster tiles the kept steps read over this tile, and how many (more than the
        // distinct ones, which the raster caches' budget is about).
        self.ranges.clear();
        let mut reads = [0u64; 4];
        for p in kept.iter().flat_map(|&i| [2 * i, 2 * i + 1]) {
            let range = self.plans[p]
                .as_ref()
                .map(|plan| (plan.format, plan.range_over(area)))
                .filter(|(_, range)| !range.is_empty());
            if let Some((format, range)) = range {
                reads[format.index()] += u64::from(range.width) * u64::from(range.height);
            }
            self.ranges.push(range.map(|(_, range)| range));
        }
        let over_budget = GpuTileFormat::ALL
            .iter()
            .any(|f| reads[f.index()] > u64::from(self.capacity[f.index()]));
        let place = [self.level as u32, col, row];
        let step_keys = kept.iter().map(|&i| self.step_keys[i]);
        // What a direct frame would do: coarser levels rather than missing layers when the tile
        // reads more raster tiles than their caches hold. The key holds the tiles of the levels
        // read in the end.
        if over_budget {
            let plans = fitted_plans(&kept, &self.plans, area, self.capacity);
            let key = tile_key(
                &self.seed,
                place,
                step_keys,
                plans
                    .iter()
                    .map(|plan| plan.as_ref().map(|plan| (plan, plan.range()))),
            );
            return KeyedTile {
                area,
                kept,
                key,
                fitted: Some(plans),
            };
        }
        let read = kept
            .iter()
            .flat_map(|&i| [2 * i, 2 * i + 1])
            .zip(&self.ranges)
            .map(|(p, range)| self.plans[p].as_ref().zip(*range));
        let key = tile_key(&self.seed, place, step_keys, read);
        KeyedTile {
            area,
            kept,
            key,
            fitted: None,
        }
    }

    /// The plans a fill of `tile` samples.
    fn plans_of(&self, tile: KeyedTile<'a>) -> Vec<Option<RasterPlan<'a>>> {
        tile.fitted
            .unwrap_or_else(|| fitted_plans(&tile.kept, &self.plans, tile.area, self.capacity))
    }
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

    // Keys of the display tiles of a document, from its steps, without a GPU: what a frame
    // does before deciding which tiles to composite.
    use slopshop_core::color::{AlphaMode, ChannelLayout, ColorSpace, PixelFormat, SampleType};
    use slopshop_core::tile::TileCoord;
    use slopshop_core::{Affine, BlendMode, Edit, Layer, LayerContent, LayerMask, LinearRgba};

    /// Raster tile caches that never limit a tile.
    const ROOMY: [u32; 4] = [1024; 4];

    // The 1024 × 512 document of most tests: 4 × 2 display tiles at level 0, 2 × 1 at level 1.
    const SIZE: Size = Size::new(1024, 512);

    fn pattern(size: Size, seed: u32) -> Arc<RasterImage> {
        let mut px = Vec::with_capacity(size.pixel_count() as usize * 4);
        for y in 0..size.height {
            for x in 0..size.width {
                px.extend([
                    (x + seed) as u8,
                    (y * 3 + seed) as u8,
                    (x ^ y) as u8,
                    200 + (seed % 50) as u8,
                ]);
            }
        }
        Arc::new(RasterImage::from_pixels(size, PixelFormat::RGBA8_SRGB, &px).unwrap())
    }

    fn gray(size: Size) -> Arc<RasterImage> {
        let format = PixelFormat {
            layout: ChannelLayout::Gray,
            sample: SampleType::U8,
            color_space: ColorSpace::LINEAR_SRGB,
            alpha: AlphaMode::Straight,
        };
        let px: Vec<u8> = (0..size.pixel_count()).map(|i| (i % 251) as u8).collect();
        Arc::new(RasterImage::from_pixels(size, format, &px).unwrap())
    }

    /// `image` with the level-0 tile (`col`, `row`) replaced by one of `value`s: the pixels of
    /// a stroke's next frame; every other tile is shared.
    fn with_tile(image: &RasterImage, col: u32, row: u32, value: u8) -> Arc<RasterImage> {
        let tile = vec![value; RasterImage::tile_bytes(image.format())];
        let replaced = vec![(TileCoord { col, row }, Arc::from(tile))];
        Arc::new(image.with_tiles(replaced).unwrap())
    }

    fn new_layer(document: &mut Document, content: LayerContent) -> Layer {
        Layer {
            style: None,
            transform: Affine::IDENTITY.into(),
            clipped: false,
            id: document.allocate_layer_id(),
            name: "layer".into(),
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask: None,
            content,
        }
    }

    /// A document of `layers`, bottom to top; they take their ids from `document`.
    fn document(mut document: Document, layers: Vec<Layer>) -> Document {
        for (index, layer) in layers.into_iter().enumerate() {
            Edit::InsertLayer {
                parent: None,
                index,
                layer,
            }
            .apply(&mut document)
            .unwrap();
        }
        document
    }

    /// A document of one raster layer, shaped by `shape`.
    fn one_raster(image: Arc<RasterImage>, shape: impl FnOnce(&mut Layer)) -> Document {
        let mut doc = Document::new(SIZE);
        let mut layer = new_layer(&mut doc, LayerContent::raster(image));
        shape(&mut layer);
        document(doc, vec![layer])
    }

    /// The keys of all the tiles of `level` (row-major) of `document`, all of it visible, with
    /// raster tile caches of `capacity`.
    fn keys_with(document: &Document, level: usize, capacity: [u32; 4]) -> Vec<ContentKey> {
        let size = document.size();
        let scene = Scene {
            document,
            steps: display_steps_with(document, None),
            visible: Some([0.0, 0.0, f64::from(size.width), f64::from(size.height)]),
            center: [0.0, 0.0],
        };
        let range = level_range(&scene, level);
        let mut planned = LevelPlan::new(&scene, level, capacity);
        let mut keys = Vec::new();
        for row in range.y..range.y + range.height {
            for col in range.x..range.x + range.width {
                keys.push(planned.keyed(col, row).key);
            }
        }
        keys
    }

    fn keys(document: &Document, level: usize) -> Vec<ContentKey> {
        keys_with(document, level, ROOMY)
    }

    /// The indices where two lists of keys differ.
    fn changed(before: &[ContentKey], after: &[ContentKey]) -> Vec<usize> {
        assert_eq!(before.len(), after.len());
        (0..before.len())
            .filter(|&i| before[i] != after[i])
            .collect()
    }

    #[test]
    fn a_new_image_sharing_every_tile_keeps_every_key() {
        let image = pattern(SIZE, 0);
        let same = Arc::new(image.with_tiles(Vec::new()).unwrap());
        assert_ne!(image.id(), same.id());
        let a = one_raster(image, |_| {});
        let b = one_raster(same, |_| {});
        assert_eq!(keys(&a, 0), keys(&b, 0));
        assert_eq!(keys(&a, 1), keys(&b, 1));
    }

    #[test]
    fn only_the_display_tiles_over_a_changed_raster_tile_change_keys() {
        let image = pattern(SIZE, 0);
        let edited = with_tile(&image, 1, 0, 7);
        let a = one_raster(image, |_| {});
        let b = one_raster(edited, |_| {});
        // Level 0: the second tile of the first row, nothing else.
        assert_eq!(changed(&keys(&a, 0), &keys(&b, 0)), [1]);
        // Level 1: the pyramid tile above it (which covers 512 pixels) changes, not the other.
        assert_eq!(changed(&keys(&a, 1), &keys(&b, 1)), [0]);
    }

    #[test]
    fn identical_pixels_in_other_tiles_are_not_assumed_equal() {
        // Keys follow the tiles' identities, not their bytes: conservative, never stale.
        let a = one_raster(pattern(SIZE, 0), |_| {});
        let b = one_raster(pattern(SIZE, 0), |_| {});
        assert_eq!(changed(&keys(&a, 0), &keys(&b, 0)).len(), 8);
    }

    #[test]
    fn a_layers_other_fields_still_change_the_keys_of_the_tiles_it_reaches() {
        // A layer of 200 × 200 pixels at (10, 10): in the first tile only.
        let image = pattern(Size::new(200, 200), 0);
        let base = |shape: fn(&mut Layer)| {
            one_raster(Arc::clone(&image), |layer| {
                layer.transform = Affine::translation(10.0, 10.0).into();
                shape(layer);
            })
        };
        let plain = keys(&base(|_| {}), 0);
        assert_eq!(plain, keys(&base(|_| {}), 0));
        let shapes: [fn(&mut Layer); 5] = [
            |l| l.opacity = 0.5,
            |l| l.blend_mode = BlendMode::Multiply,
            |l| l.transform = Affine::translation(11.0, 10.0).into(),
            |l| {
                l.transform = Affine::scale(1.1, 1.1)
                    .then(Affine::translation(10.0, 10.0))
                    .into()
            },
            |l| {
                l.mask = Some(LayerMask {
                    original: None,
                    image: gray(Size::new(200, 200)),
                    enabled: true,
                    replaces_alpha: false,
                });
            },
        ];
        for (i, shape) in shapes.into_iter().enumerate() {
            assert_eq!(
                changed(&plain, &keys(&base(shape), 0)),
                [0],
                "field change {i}"
            );
        }
    }

    #[test]
    fn a_resampled_layers_keys_change_within_the_filters_support() {
        // A half-pixel shift: resampled, each pixel reads texels around it, so the tiles next to
        // a changed raster tile read it too.
        let image = pattern(SIZE, 0);
        let edited = with_tile(&image, 1, 0, 7);
        let shift = |l: &mut Layer| l.transform = Affine::translation(0.5, 0.5).into();
        let a = one_raster(image, shift);
        let b = one_raster(edited, shift);
        let moved = changed(&keys(&a, 0), &keys(&b, 0));
        // Row-major, 4 columns: the tile over it (1), and its neighbours, which read it; not the
        // far ones.
        assert!(moved.contains(&1), "{moved:?}");
        assert!(moved.contains(&0) && moved.contains(&2), "{moved:?}");
        assert!(!moved.contains(&3) && !moved.contains(&7), "{moved:?}");
        // Whole-pixel offsets read no more than the tile placed over them.
        let straight = |l: &mut Layer| l.transform = Affine::translation(256.0, 0.0).into();
        let image = pattern(SIZE, 0);
        let edited = with_tile(&image, 0, 0, 7);
        let a = one_raster(image, straight);
        let b = one_raster(edited, straight);
        assert_eq!(changed(&keys(&a, 0), &keys(&b, 0)), [1]);
    }

    #[test]
    fn a_changed_mask_tile_changes_only_the_tiles_it_covers() {
        let mask = gray(SIZE);
        let edited = with_tile(&mask, 2, 1, 9);
        let masked = |mask: Arc<RasterImage>| {
            let mut doc = Document::new(SIZE);
            let mut layer = new_layer(
                &mut doc,
                LayerContent::Fill {
                    color: LinearRgba::new(0.9, 0.2, 0.1, 1.0),
                },
            );
            layer.mask = Some(LayerMask {
                original: None,
                image: mask,
                enabled: true,
                replaces_alpha: false,
            });
            document(doc, vec![layer])
        };
        // Row-major, 4 columns: (2, 1).
        assert_eq!(
            changed(&keys(&masked(mask), 0), &keys(&masked(edited), 0)),
            [6]
        );
    }

    #[test]
    fn the_paint_tiles_of_a_stack_are_in_the_keys_of_the_tiles_reading_them() {
        use slopshop_core::BlendSpace;
        use slopshop_core::stack::{LayerStack, PaintEntry, PaintOp};
        let original = pattern(SIZE, 0);
        let empty = PaintEntry::empty(original.format(), SIZE, BlendSpace::Perceptual);
        let red = PaintOp::Color(LinearRgba::new(1.0, 0.0, 0.0, 1.0));
        let paint = |tiles: &[(u32, u32)]| {
            let tiles = tiles
                .iter()
                .map(|&(col, row)| {
                    let coord = TileCoord { col, row };
                    (
                        coord,
                        empty.painted_tile(coord, red, |x, _| x as f32 / 256.0),
                    )
                })
                .collect();
            Arc::new(empty.with_tiles(tiles).unwrap())
        };
        // Its pixels are not evaluated: the display evaluates the stack (ADR 0029) and reads the
        // paint's tiles.
        let stacked = |paint: Arc<PaintEntry>| {
            let stack = LayerStack::new(Arc::clone(&original))
                .with_top_paint(paint)
                .unwrap();
            let mut doc = Document::new(SIZE);
            let layer = new_layer(&mut doc, LayerContent::raster(Arc::clone(&original)));
            let id = layer.id;
            let mut doc = document(doc, vec![layer]);
            Edit::SetLayerStack {
                id,
                stack,
                shown: None,
            }
            .apply(&mut doc)
            .unwrap();
            doc
        };
        let before = stacked(paint(&[(0, 0)]));
        let after = stacked(paint(&[(0, 0), (3, 1)]));
        let moved = changed(&keys(&before, 0), &keys(&after, 0));
        assert!(moved.contains(&7), "{moved:?}");
        // A paint entry makes its images once (and its identity tiles with them: a later entry
        // makes its own, so keys elsewhere change too, as before: conservative). The same paint
        // gives the same keys.
        let paint = paint(&[(0, 0), (3, 1)]);
        assert_eq!(
            keys(&stacked(Arc::clone(&paint)), 0),
            keys(&stacked(paint), 0)
        );
    }

    #[test]
    fn tiles_read_at_a_coarser_level_because_of_the_budget_are_in_the_key() {
        // Two layers over each display tile, raster tile caches of one tile: a tile is
        // composited from the coarsest level, a single tile that a change anywhere in the
        // image changes, whatever the level-0 tile under the display tile.
        let a = pattern(SIZE, 0);
        let b = pattern(SIZE, 1);
        let two = |a: Arc<RasterImage>| {
            let mut doc = Document::new(SIZE);
            let first = new_layer(&mut doc, LayerContent::raster(a));
            let mut second = new_layer(&mut doc, LayerContent::raster(Arc::clone(&b)));
            second.opacity = 0.5;
            document(doc, vec![first, second])
        };
        let tiny = [1; 4];
        let before = keys_with(&two(Arc::clone(&a)), 0, tiny);
        let after = keys_with(&two(with_tile(&a, 1, 0, 7)), 0, tiny);
        assert!(changed(&before, &after).contains(&0));
        // With room for the tiles, the first tile does not read the changed one.
        let before = keys(&two(Arc::clone(&a)), 0);
        let after = keys(&two(with_tile(&a, 1, 0, 7)), 0);
        assert_eq!(changed(&before, &after), [1]);
    }

    #[test]
    fn a_cached_display_tile_keeps_allocated_the_raster_tiles_its_key_holds() {
        // Keys hold the addresses of raster tiles: while a tile is cached, no other tile can
        // take the address of one of them. (A raster tile cache of one tile lets go of all
        // but the last one at once: only the display cache keeps the others.)
        let r = match crate::Renderer::new() {
            Ok(r) => r.with_tile_capacity(1),
            Err(e) if std::env::var("SLOPSHOP_REQUIRE_GPU").as_deref() == Ok("1") => {
                panic!("GPU required but unavailable: {e}")
            }
            Err(e) => {
                eprintln!("skipping GPU test: {e}");
                return;
            }
        };
        let image = pattern(SIZE, 0);
        let weak: Vec<_> = image.levels()[0]
            .tiles()
            .iter()
            .map(Arc::downgrade)
            .collect();
        assert_eq!(weak.len(), 8);
        let doc = one_raster(Arc::clone(&image), |_| {});
        let view = ViewTransform {
            origin: [0.0, 0.0],
            scale: 1.0,
        };
        let stats = r.profile_view(&doc, view, SIZE, false).unwrap();
        assert_eq!(stats.tiles_composited, 8);
        drop(doc);
        drop(image);
        assert!(weak.iter().all(|tile| tile.strong_count() > 0));
    }
}
