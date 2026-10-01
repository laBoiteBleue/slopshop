# Architecture

Status: early foundations. Planned work: [roadmap.md](roadmap.md). This document describes what exists **and** the constraints the
design must keep satisfiable. Anything not implemented is marked as such.

## Overview

```
┌──────────────────────────── app (Tauri 2) ────────────────────────────┐
│  UI: Svelte 5 + TypeScript (webview)                                   │
│    presentation, input, i18n — no image logic                          │
│          │ intents (JSON: small DTOs)        ▲ frames (raw binary)      │
│          ▼                                   │ viewport-sized only      │
│  app/src-tauri: IPC commands + DTOs (thin, async, no image logic)      │
└──────────┬─────────────────────────────────────────────────────────────┘
           │ Rust calls
┌──────────▼───────────┐     ┌──────────────────────┐    ┌──────────────┐
│ slopshop-render      │────▶│ slopshop-core        │◀───│ slopshop-cli │
│ wgpu, headless       │     │ document, edits,     │    │ headless     │
│ view → frame         │     │ history, geometry,   │    └──────────────┘
└──────────────────────┘     │ color; no deps       │
                             └──────────▲───────────┘
                             ┌──────────┴───────────┐
                             │ slopshop-io          │
                             │ import, export       │
                             └──────────────────────┘
```

Dependency direction is strict: `core` ← (`render`, `io`) ← (`cli`, `app`). `render` and `io`
do not depend on each other: export receives its pixel source as a closure (see
[Export data flow](#export-data-flow)).

## Crates

### `slopshop-core` (implemented, minimal)

- `geom`, `tile`: `u32` pixel coordinates with `u64` edges/counts; tile grid geometry and
  region → tiles queries. Basis for regions of interest (ROI), caches and out-of-core storage.
- `color`: `ColorSpace` = RGB primaries + white point + transfer function (sRGB, gamma, ICC
  parametric, PQ, HLG…), conversion matrices with Bradford adaptation, the working space
  (linear Rec.2020, [ADR 0007](adr/0007-color-management.md)), `PixelFormat`, f16 conversion.
- `raster`: immutable tiled images (256 px tiles in the source format, shared via `Arc`) with a
  display pyramid ([ADR 0005](adr/0005-pixel-storage-v0.md)). An image smaller than its canvas
  (`from_placed`) shares one tile, and its pyramid tiles, for the uniform area around it.
- `document`: a layer tree, bottom to top, addressed by stable `LayerId`s that are never
  reused. Layers are procedural *fills*, *rasters*, *adjustments* of what is below them
  ([ADR 0020](adr/0020-adjustment-layers.md)) or *groups* of other layers
  ([ADR 0015](adr/0015-layer-groups.md)), each with an opacity, a blend mode, a transform to its parent
  ([ADR 0017](adr/0017-non-destructive-transforms.md)) and an optional
  mask ([ADR 0014](adr/0014-layer-masks.md)); the document has a blend space. `revision`
  increases on every change.
- `resample`: how a transformed layer is sampled ([ADR 0018](adr/0018-resampling.md)): EWA
  with a Jinc-windowed Jinc kernel and anti-ringing, from the pyramid level matching the scale;
  computed per layer once, shared by the CPU compositor and the GPU (same kernel table).
- `thumbnail`: small previews of rasters for the UI, read from the coarsest pyramid level
  that is large enough and converted like an 8-bit sRGB export.
- `blend`: blend modes and blend spaces ([ADR 0012](adr/0012-blend-modes.md)), the reference
  math that the CPU compositor uses and the GPU shader mirrors.
- `edit`: `Edit` is the only mutation path. Edits are validated (failure leaves the document
  untouched) and return their exact inverse.
- `selection`: the document's selection as a 16-bit coverage mask sharing its uniform tiles
  ([ADR 0024](adr/0024-selections.md)): shapes rasterized exactly, combine modes, feather,
  inverse, bounds, and the outline (marching ants) at a pyramid level over a region.
- `session`: document + linear undo/redo history made of inverse edits. *Gestures* (e.g. a
  slider drag) apply edits live and are recorded as one entry (`Edit::Batch`).
- `view`: `ViewTransform` mapping output pixels to document pixels, and `Viewport`: fit mode,
  zoom around a point, preset zoom steps, pan, and a clamp that keeps part of the document
  visible. View state is never part of the undo history.
- Export support ([ADR 0008](adr/0008-export.md)): `composite`, the CPU reference compositor
  (full resolution, unclipped; the GPU's test oracle and fallback), whose `steps` flatten the
  visible layer tree into one pass with a stack of accumulators (leaving out layers hidden by an
  opaque layer above them), shared with the GPU shader
  ([ADR 0015](adr/0015-layer-groups.md)); `convert`, the only
  conversion from the working space to a file's pixel format (matrix, luminance for gray
  targets, alpha, range, transfer, exact quantization, blue-noise dither from `blue_noise`),
  counting every lossy event; `job`,
  cancellation and progress.

### `slopshop-render` (implemented, minimal)

A wgpu compute pipeline composites visible layers in linear light (premultiplied alpha) over a
checkerboard and encodes sRGB for display, for exactly the output-sized area. Raster layers are
sampled from a GPU tile cache (texture array, LRU) at the pyramid level matching the zoom; only
visible tiles are uploaded, and layers without any are left out of the frame. Transformed layers are resampled like the CPU does (zoomed in, at
document pixels, so the view shows what export writes). The display
encoding is a *view transform*; the document is never converted. Headless: no surface needed.
Viewport frames go through a display cache ([ADR 0022](adr/0022-display-cache.md)): composited
tiles of the document's power-of-two levels, kept on the GPU in half floats (linear display
space) and addressed by a hash of everything their compositing reads; a frame composites only the
visible tiles the cache lacks, each from the layers that reach it, then presents the view from the
cached tiles. Views needing more tiles than the cache holds are composited directly.
`SLOPSHOP_DISPLAY_CACHE=0` turns the cache off. Native presents are progressive: a frame
composites a bounded amount of missing tiles, nearest the view's center first, shows the rest
from a coarser level with few tiles, and tells the UI to present again until the view is complete.
For export, `Renderer::render_region` runs the same compositing code on a document region at
full resolution (level 0 always) and reads back the working-space values as premultiplied RGBA
f32, unclipped, in chunks sized to the tile and GPU buffer limits. `export_source` wraps it as
export's pixel source, with the CPU compositor when there is no renderer or when a region shows
more raster images than the GPU tile cache holds.

### `slopshop-io` (implemented, minimal)

Decodes files into `RasterImage` in their native precision (`image` codecs, `tiff` directly, an
in-house PSD/PSB reader in `psd`), with an in-house matrix/TRC ICC reader and EXIF orientation.
`open_file` opens a layered file (Photoshop) as a `Document` with warnings per layer, anything
else as an image; `open_image` always gives an image (a PSD's flattened composite). Formats not supported yet are recognized
and refused with an explicit reason; see [ADR 0006](adr/0006-universal-import-and-licensing.md).

Export (`export`) writes PNG (8/16-bit), TIFF (8/16-bit, 32-bit float; BigTIFF when needed),
OpenEXR (half/float), JPEG (`jpeg-encoder`) and WebP (lossless: `image-webp`; lossy: libwebp
through `export::webp::ffi`, the crate's only `unsafe` module), always color-tagged, with
per-format defaults (`default_spec`), size limits (`max_side`) and an in-house ICC writer; see
[Export data flow](#export-data-flow) and [ADR 0010](adr/0010-jpeg-webp-export.md).

Documents (`slop`) are saved as `.slop` files ([ADR 0009](adr/0009-document-file-format.md),
[specification](file-format.md)): content-addressed tiles compressed in parallel, a JSON
manifest, an append-only log with two commit slots, so a save only writes what changed and a
crash leaves the previous save readable. `SlopFile` keeps what the file already holds between
saves. Full writes (first save, Save As, compaction, exports) share `atomic::TempFile`.

### `slopshop-raw` (implemented, basic)

A separate executable the importer runs for each camera RAW file
([ADR 0023](adr/0023-camera-raw-helper.md)): rawler (LGPL-2.1, used by this crate only)
decodes and demosaics, our steps apply the as-shot white balance and the camera matrix to
linear Rec.2020, and the image goes to the importer through a pipe. Keeps rawler replaceable
and a decoder crash out of the editor.

### `slopshop-cli` (implemented, minimal)

`slopshop gpu`, `slopshop render`, `slopshop export` (a `.slop` document or an image file,
opened as a one-layer document, exported with the format defaults and optional overrides),
`slopshop save` (images to a `.slop` document, one layer each), `slopshop inspect` and
`slopshop bench` (viewport frames of a document redrawn, panned and zoomed, timed on the CPU
and, through timestamp queries, on the GPU); `--bench` prints timings.
Proves the engine runs without the UI.

### `app` (implemented, minimal)

Tauri shell + Svelte UI laid out like Photoshop ([ADR 0013](adr/0013-familiar-layout.md)): a
menu bar (File, Edit, Image, Layer, View, Help) with Photoshop's shortcuts, viewport, layer panel (visibility, live opacity, rename, drag to
reorder, delete, add fill, blend mode, document blend space, thumbnails), undo/redo, Save / Save As of `.slop` documents (asking before
unsaved changes are lost), FR/EN interface. The IPC client serializes mutations so
they reach the engine in order (Tauri runs async commands concurrently). Commands are `async` (never on the main thread); GPU work runs in
`spawn_blocking`. The shell owns the open documents (one per tab, each with its own history and
view state `Viewport`); every request names its document, and requests for a closed document are
rejected. Frames are returned as
`tauri::ipc::Response` (an `ArrayBuffer` in JS): a 40-byte header (size, fit flag, document
revision, zoom, engine render time) followed by the pixels, parsed without copy in `engine.ts`.

Export (`src-tauri/src/export.rs`, ADR 0008): `export_defaults`, `export_spaces` and
`export_max_side` give the export dialog the settings and limits of a format; `export_document` snapshots the document (raster pixels
are shared, not copied), starts a job on a `spawn_blocking` worker (never the main thread) and
returns the job id at once; `cancel_export` cancels a job by id. Jobs report through the events
`export-started`, `export-progress` (throttled), `export-finished` (with the ids of the report's
notices) and `export-failed` (with an error code). Several jobs can run at once: the UI shows
the progress of each, and each outcome until it is dismissed (a success without notices, and
an error, disappear after a few seconds). "Show in folder" reveals a finished file through
`reveal_in_folder` (tauri-plugin-opener, not exposed to JavaScript). Closing the main window while jobs run cancels them and waits,
off the main thread and a few seconds at most, for them to remove their temporary files.

## Export data flow

[ADR 0008](adr/0008-export.md). Export reads the document (no edit) and streams it:

```
document ──▶ pixel source ──▶ band channel ──▶ convert ──▶ format writer ──▶ .<name>.<pid>-<n>.slopshop-tmp
             (render: GPU      (capacity 1)    (core,      (PNG stream;         │ sync, rename
             region, or CPU                     rows in     TIFF/EXR: parallel   ▼
             compositor)                        parallel;   compression; JPEG:  <name>
                                                matte for   encoder thread;
                                                no alpha)   WebP: whole frame)
```

- Full-width bands of 256 rows, pyramid level 0 (resampled layers: the level their scale
  needs, ADR 0018): premultiplied RGBA f32 in the working
  space, finite values unclipped (NaN → 0 and ±inf → ±65504 by the source, counted). At most 3
  source bands in memory (produced, queued, converted), so memory depends on the width only.
- `core::convert` is the only place where values change format; every lossy event (clipping,
  half-float overflow, non-finite values, flattened alpha, color dropped by a gray export) is
  counted into an `ExportReport` of stable ids.
- Cancellation (`CancelToken`) is checked between bands and progress is reported per band. On
  error or cancellation the temporary file (unique to the job) is deleted and the destination
  is untouched.
- The GPU source works in chunks of at most 128 MiB of output; GPU out-of-memory or validation
  errors are captured and the export continues on the CPU compositor.

## Key invariants

1. **Engine owns the document.** The UI holds only views (DTOs) and sends intents.
2. **Every document change is an `Edit`**, hence undoable. Initial content built at load time
   may bypass history by applying edits directly to the document.
3. **Ids, not indices**, identify things across time and across the IPC.
4. **Explicit pixel formats and color spaces**; conversions are named functions; clipping only
   happens at the display boundary and in export (`core::convert`, counted and reported).
5. **Nothing assumes a whole image fits in RAM/VRAM**; rendering is driven by the requested
   output region.
6. **IPC carries small DTOs and viewport-sized frames only**, and identifiers rather than
   display text (the UI translates).

## Designed for, not implemented yet

- **Pixel layers** stored as tiles (see `tile`), with copy-on-write sharing so that document
  snapshots and undo do not duplicate pixels, and eviction to disk for images larger than RAM.
- **Mip levels** of tiles for zoomed-out views, and caches keyed by (node, region, level,
  revision) for partial recomputation.
- **Operation nodes** (adjustments, filters, AI generations) and the evolution of the stack
  into a **DAG**; see [ADR 0003](adr/0003-document-model-edits-history.md).
- **AI nodes** with parameters, prompt, model + version, seed, mask, source region,
  dependencies and cached results, with invalidation instead of automatic recomputation; see
  [research notes](research/hd-generative-ai.md).
- **Color management** beyond the two built-in spaces (ICC profiles, OCIO): open question.

## Open questions (hard to change later)

| Topic                         | Current choice                          | Decide before                     |
| ----------------------------- | --------------------------------------- | --------------------------------- |
| Viewport presentation         | Native surface on Windows, frames over IPC elsewhere ([ADR 0002](adr/0002-viewport-frame-transport.md)) | interactive tools (brush, pan/zoom at 60 fps) |
| Document model (stack vs DAG) | Stack of layers with stable ids         | first non-trivial node type       |
| Color management              | Named spaces (linear sRGB working)      | first real image import           |
| Tile storage / out-of-core    | Geometry only                           | pixel layers                      |
| HD generative AI strategy     | Research only                           | first AI feature                  |
| File format                   | `.slop` v0 ([ADR 0009](adr/0009-document-file-format.md)), frozen at 1.0 | 1.0 |
