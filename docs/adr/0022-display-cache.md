# 0022 — Display cache: composited tiles addressed by their content

Status: **accepted** (2026-10-01, maintainer decision: content addressing, Photoshop-like
filtering between power-of-two zooms, progressive refinement, an f16 cache).

## Context

The maintainer's target is display performance on par with Photoshop: navigation and live
adjustments that stay fluid whatever the document (hundreds of layers, 200+ MP canvases).

Today every viewport frame composites **every visible step for every output pixel**
(`composite.wgsl`, `main`): nothing composited is kept. Moving the view by one pixel costs as
much as the first frame; so does presenting the same view again. Culling (hidden layers,
layers without visible tiles) and the GPU make it fast in simple cases (`slopshop bench`:
about 1–2 ms per 1080p frame on an RTX 5070 Ti for a 233 MP image with a 346-slice DICOM
series), but the cost still grows with *output pixels × steps*: a few dozen visible layers with
masks or resampling, a 4K or 8K viewport, or a modest GPU bring it back to tens or hundreds of
milliseconds per frame. Photoshop's answer, and that of most large-image editors, is a cache of
composited tiles at several resolutions, recomputed only where something changed.

Constraints from CLAUDE.md and earlier ADRs: performance first; never assume the document fits
in VRAM; display transforms are views (the document is never converted); exactness at 100 %
(export and view agree, ADR 0018); the stack must be able to become a DAG; the engine stays
usable headless; `unsafe` and new dependencies need a decision.

## Decision

1. **The renderer keeps a cache of composited document tiles**, 256 × 256 texels, at the
   power-of-two levels of the document pyramid (level *L*: one texel = 2^*L* × 2^*L* document
   pixels, layers sampled at their level *L* as today). Stored on the GPU as **premultiplied
   linear RGBA16Float in the display space** (linear sRGB today) in texture arrays, under a VRAM
   budget, least recently used tiles evicted first. Nothing is cached on the CPU.
   (Implementation note, 2026-10-01: the decision first said *working space*. Clamping
   working-space values to the half-float range changes how extreme values look once converted
   to the display, e.g. −inf in one channel no longer shows as black; after the display matrix,
   a value beyond ±65504 clips on the display exactly as before. The display transform is linear,
   so filtering cached tiles is still filtering the composite, and it is part of the key.)

2. **A tile's key is its content, not its position in time**: (level, column, row, a 128-bit
   hash of everything the shader reads for that tile). The hash covers the document's blend
   space and the encoded fields of every step that can change the tile (the same bytes uploaded
   to the shader, tile slots excluded), with the ids of the images and masks it samples
   (immutable, ADR 0005), the display transform and the raster tile budget (which may coarsen
   the levels read). Each step is hashed once per frame, each tile combines the hashes of its
   steps: a frame whose tiles are cached costs well under a millisecond of CPU time even with
   hundreds of layers. A tile is valid exactly when its key is found: **there is no
   invalidation to get right** — any edit, undo or redo changes the steps of the tiles it
   touches and only those. Undoing reuses the tiles of the previous state while they are still
   in the cache. Completeness of the hash is by construction (it hashes what the GPU receives),
   and checked by tests that change each field.

3. **Per tile, only the steps that reach it** are encoded (binning): a step's reach is its
   placed image (whole-pixel offset), its transformed bounds plus the resampling support, or
   the whole canvas (fills, adjustments, masks restrict nothing conservatively); groups without
   reaching content are dropped for that tile. The hash and the shader both use that list, so a
   layer far from a tile neither invalidates it nor costs anything there.

4. **A frame becomes two passes**:
   - *fill*: render the visible tiles of the chosen level that are missing (the existing
     compositing code, with one output texel per level texel and the box footprint of the
     level), within a per-frame budget;
   - *present*: for each output pixel, filter the cached tiles of that level (the same box
     footprint as `sample_raster`, at most 2 × 2 texels when zoomed out, the texel itself at
     100 % and when zoomed in), then apply the display transform, the checkerboard and the
     pasteboard edge, and write the swapchain (Windows) or the frame (other platforms). This pass
     costs the same whatever the document.

5. **Progressive refinement**: a visible tile that is missing at the chosen level is shown from
   the finest coarser level already cached (upscaled), and the frame reports itself
   *incomplete*; the UI then asks for another present until the view is complete. Navigation
   never waits for compositing; a huge document shows a soft image for a frame or two.

6. **Live gestures** (slider drags, transforms) keep working as now: the changed steps give new
   keys every frame, so the visible tiles they reach are composited again, and only those. A
   later step (not part of this decision) can cache *partial* stacks — the accumulator below
   the edited layer — with the same content addressing.

7. **Export is unchanged**: it renders regions at full resolution in f32, unclipped, through
   `render_region` (ADR 0008); the f16 display cache is never a source for files.

## Behavior that changes

- At 100 % with whole-pixel panning, and at every power-of-two zoom aligned on the level grid,
  the displayed pixels are the same as today up to f16 rounding (11-bit precision, invisible on
  an 8-bit display; values beyond ±65504 are clipped by the display anyway).
- **Between power-of-two zooms** (e.g. 66.7 %) the view is a box filter of the next finer
  level's composite instead of the composite of box-filtered layers: very slightly softer, as in
  Photoshop. (Arguably more correct: blending is non-linear, so filtering after compositing is
  what a downscaled export shows.)
- During fast navigation on large documents, newly exposed areas may show a coarser level
  before refining (Photoshop does the same).

## Alternatives

- **Keep compositing everything every frame, and optimize the shader** (binning per screen
  tile, specialized pipelines). Useful and partly included (point 3), but the cost still scales
  with output pixels × steps on every frame, including frames that change nothing.
- **Invalidate by dirty regions reported by each edit**: the classic approach. Every `Edit`
  (and every future node type) must report exactly what it changes, before and after; a missed
  case shows stale pixels. Undo and redo cannot reuse earlier tiles. Content addressing gets
  both for free and stays correct when the model becomes a DAG.
- **Cache in RGBA32Float**: exact working-space values, but twice the VRAM and bandwidth for a
  display-only cache whose output is 8-bit (or 10-bit HDR later, which f16 still covers).
- **Cache at the screen's exact zoom** (re-render on every zoom change, reuse only while
  panning): simpler presentation, sharp at every zoom, but zooming composites everything again
  and nothing is shared between zoom steps.
- **Cache on the CPU / in RAM**: larger capacity, but every present would upload tiles to the
  GPU; the bandwidth is what the cache is meant to save.

## Consequences

- New in `slopshop-render`: a composite tile cache (texture arrays, LRU, budget), the per-tile
  step lists and their hashes, a fill entry point writing to a cache slot, a present entry point
  reading cached tiles. The `Renderer` API stays `render_view` / `present_view` (+ an
  *incomplete* flag); the app adds one re-present request when a frame is incomplete.
- `slopshop-core`: the reach of a step (already needed by the culling of ADR 0015's step list)
  becomes shared; no new dependency anywhere (the 128-bit hash is a small in-house function or
  `std` hashing run twice with different keys — to settle during implementation).
- VRAM: 512 KiB per cached tile; a 4K viewport needs about 160 tiles at one level (80 MiB). The
  budget follows the adapter's memory, with a fixed default until wgpu reports it.
- Tests: GPU tests compare the cached display with today's direct composite at 100 % and at
  power-of-two zooms (equal up to f16 rounding), check that every encoded field changes the
  hash, that an edit recomposites only the tiles it reaches (counted by `FrameStats`), and that
  undo reuses cached tiles. `slopshop bench` gets a scenario for the same view presented again
  and for a slider-like change of one layer.
- Steps: (1) per-tile step lists and keys, measured; (2) fill + present passes on Windows, with
  the direct path kept behind a switch for comparison; (3) progressive refinement and the UI's
  re-present; (4) partial stacks for gestures. Each step is benchmarked against the previous.
