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
  inverse, bounds, and the outline (marching ants) at a pyramid level over a region (drawn by the
  UI for frames over IPC; the GPU draws the ants itself where the engine presents natively,
  `slopshop_render::Ants`). Saved
  selections are named objects of the document (`Document::saved_selections`), kept in `.slop`.
  The color-comparing tools (Magic Wand, Grow, Similar, Color Range) take their composited
  pixels from an injected `PixelSource` (the GPU export path in the app, the CPU compositor
  otherwise and as fallback).
- `liquify` (ADR 0037): Filter > Liquify's displacement field (a sparse tiled grid of one node
  per 1, 2 or 4 pixels, aligned with the layer's tiles, with a freeze mask), the brush tools
  that compose exact shifts into it, the warp that evaluates a layer, a crop at a pyramid level
  or a frame of the workspace through it. A Liquify entry of a layer's stack (`stack`) holds
  the field and is a materialization point like a filter.
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

### `slopshop-ai` (in progress)

The AI helper ([ADR 0025](adr/0025-ai-selection.md)). The library holds the protocol (binary
frames on the helper's standard input and output) and the client the app starts it with; it
needs nothing. The `slopshop-ai` executable (feature `helper`) loads ONNX Runtime at run time
from a library the app downloads (never linked, never shipped with the editor), runs on
DirectML (any graphics card on Windows), Core ML (macOS on Apple silicon) or the processor
(Linux), and runs SAM 2.1: an image is encoded once, then each prompt (points, a
box) is decoded in milliseconds into 256² logits, which `selection::select_logits` turns into a
selection over the canvas. A model's crash or its memory stays out of the editor.

Its `install` module (feature `install`) downloads ONNX Runtime and the models on the user's
consent into a per-user folder. A manifest generated by `tools/manifest.py` pins every file
(URL, size, SHA-256, license). A library inside an archive (ONNX Runtime's DirectML package)
is fetched alone, as an HTTP range over its compressed bytes, and inflated (the runtime: 16 MB
to download); one inside a `.tar.gz` (ONNX Runtime's macOS and Linux releases) comes with its
whole archive (11 to 42 MB).
Downloads resume, and a file takes its name only once verified.

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

Liquify (`src-tauri/src/liquify.rs`, ADR 0037): the workspace's session lives in the document
(`OpenDocument::liquify`, shared with the commands so that they do not hold the documents'
lock while they work): `liquify_open` evaluates what the layer shows (or what a Liquify entry
is applied to) and starts from an empty field or the entry's; `liquify_stroke` takes pieces of
a stroke (the pointer's samples and the time it stayed still), `liquify_undo` and
`liquify_restore_all` walk the field's own history, `liquify_frame` returns the layer seen
through the field as raw 8-bit RGBA, and `liquify_commit` (OK) makes the field one undo entry
while `liquify_close` leaves the document untouched.

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

Updates (`src-tauri/src/update.rs`, ADR 0039): tauri-plugin-updater, registered only when the
bundle configuration sets it up (release builds; on Linux, the AppImage), driven from Rust and
not exposed to JavaScript. `update_supported`, `update_check` (the manifest at the `updates`
pre-release), `update_install` (download with progress over a `Channel`, signature check, the AI
helper stopped, install, restart) and `update_cancel`. The UI checks quietly after startup, at
most once a day (`app/src/lib/updates.ts`), and offers what it finds in the menu bar.

AI selection (`src-tauri/src/segment.rs`, ADR 0025): the `slopshop-ai` helper is started on
first use and kept, the model loaded. The image SAM sees (the document, or the view when zoomed
in, at most 1024 pixels on a side, rendered by the GPU) is encoded once and reused while only
the selection changes, so Object Selection's hover (`ai_object_hover`: a 256² mask the UI tints)
and its click or box (`ai_object_select`) only decode, in milliseconds. The
mask becomes a selection through `selection::select_logits` (specks and pinholes dropped).
Select > Subject (`ai_select_subject`) has BiRefNet find the main subject on the whole document
(1024² logits) and goes through the same path. Refine Edges (an option of the tools, and Select and Mask's edge detection, `ai_refine_base`) plans windows
along the outline with `selection::plan_refinement` (512 pixels, at most 40; a longer outline
is seen coarser), renders each, has ViTMatte (B on a GPU, S on the CPU) matte it
with a trimap that leaves undecided a band around the outline (`RefineBand`: narrow inward,
wide outward) and every partly covered pixel, and blends the overlapping windows' mattes there
only. Select > Subject keeps BiRefNet's probabilities as coverage
(`selection::select_logits_soft`). AI requests carry a task id: their progress is the
`ai-progress` event and `ai_cancel` stops them at their next step. The helper unloads BiRefNet
before ViTMatte and the reverse, and stops after two minutes unused (`stop_if_idle`).

Quick Selection (`selection::quick_select`, ADR 0026) has no model: the region in view is
rendered by the GPU (at most 1600 pixels on a side), `slopshop_core::quick_select` cuts it
between a color model of the stroke and one of the rest (a max-flow on the pixel grid, coarse
then in a band), and the changed pixels join the selection through `selection::select_scores`.

AI components (`src-tauri/src/ai.rs`, ADR 0025): `ai_components` lists what a feature needs on
this machine (ONNX Runtime with DirectML, and the models) or, for Edit > Preferences, everything this machine can use; `ai_install`
downloads on a worker with progress over a channel, one install at a time, cancelled by
`ai_cancel_install`; `ai_remove` deletes; `ai_open_license` opens only a manifest's license.
The UI asks first: sizes, licenses, and an explicit acceptance of the ones that are not
permissive open source. Files go to `<local app data>/ai`. AI is offered on Windows x64
(DirectML), macOS on Apple silicon (Core ML) and Linux x64 and ARM (processor); `ai_runtime`
tells the UI which (Refine Edges is off by default on the processor).

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
- **Sources** ([ADR 0040](adr/0040-sources.md)): layers reference read-only sources (images,
  later vector content, embedded documents, linked files, AI results) kept once per document;
  the layer tree plus these references is the document's graph, without cycles by
  construction.
- **AI nodes** with parameters, prompt, model + version, seed, mask, source region,
  dependencies and cached results, with invalidation instead of automatic recomputation; see
  [research notes](research/hd-generative-ai.md).
- **Color management** beyond the two built-in spaces (ICC profiles, OCIO): open question.

## Open questions (hard to change later)

| Topic                         | Current choice                          | Decide before                     |
| ----------------------------- | --------------------------------------- | --------------------------------- |
| Viewport presentation         | Native surface on Windows, frames over IPC elsewhere ([ADR 0002](adr/0002-viewport-frame-transport.md)) | interactive tools (brush, pan/zoom at 60 fps) |
| Document model (stack vs DAG) | Tree of layers referencing read-only sources ([ADR 0040](adr/0040-sources.md)) | settled |
| Color management              | Named spaces (linear sRGB working)      | first real image import           |
| Tile storage / out-of-core    | Geometry only                           | pixel layers                      |
| HD generative AI strategy     | Research only                           | first AI feature                  |
| File format                   | `.slop` v0 ([ADR 0009](adr/0009-document-file-format.md)), frozen at 1.0 | 1.0 |
