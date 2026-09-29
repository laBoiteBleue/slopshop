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
                             └──────────────────────┘
```

Dependency direction is strict: `core` ← `render` ← (`cli`, `app`).

## Crates

### `slopshop-core` (implemented, minimal)

- `geom`, `tile`: `u32` pixel coordinates with `u64` edges/counts; tile grid geometry and
  region → tiles queries. Basis for regions of interest (ROI), caches and out-of-core storage.
- `color`: `ColorSpace` = RGB primaries + white point + transfer function (sRGB, gamma, ICC
  parametric, PQ, HLG…), conversion matrices with Bradford adaptation, the working space
  (linear Rec.2020, [ADR 0007](adr/0007-color-management.md)), `PixelFormat`, f16 conversion.
- `raster`: immutable tiled images (256 px tiles in the source format, shared via `Arc`) with a
  display pyramid ([ADR 0005](adr/0005-pixel-storage-v0.md)).
- `document`: a layer stack, bottom to top, addressed by stable `LayerId`s that are never
  reused. Layers are procedural *fills* or *rasters*. `revision` increases on every change.
- `edit`: `Edit` is the only mutation path. Edits are validated (failure leaves the document
  untouched) and return their exact inverse.
- `session`: document + linear undo/redo history made of inverse edits. *Gestures* (e.g. a
  slider drag) apply edits live and are recorded as one entry (`Edit::Batch`).
- `view`: `ViewTransform` mapping output pixels to document pixels, and `Viewport`: fit mode,
  zoom around a point, preset zoom steps, pan, and a clamp that keeps part of the document
  visible. View state is never part of the undo history.

### `slopshop-render` (implemented, minimal)

A wgpu compute pipeline composites visible layers in linear light (premultiplied alpha) over a
checkerboard and encodes sRGB for display, for exactly the output-sized area. Raster layers are
sampled from a GPU tile cache (texture array, LRU) at the pyramid level matching the zoom; only
visible tiles are uploaded. The display
encoding is a *view transform*; the document is never converted. Headless: no surface needed.

### `slopshop-io` (implemented, minimal)

Decodes files into `RasterImage` in their native precision (`image` codecs, `tiff` directly), with
an in-house matrix/TRC ICC reader and EXIF orientation. Formats not supported yet are recognized
and refused with an explicit reason; see [ADR 0006](adr/0006-universal-import-and-licensing.md).

### `slopshop-cli` (implemented, minimal)

`slopshop gpu` and `slopshop render`. Proves the engine runs without the UI.

### `app` (implemented, minimal)

Tauri shell + Svelte UI: viewport, layer panel (visibility, live opacity, rename, drag to
reorder, delete, add fill), undo/redo, FR/EN interface. The IPC client serializes mutations so
they reach the engine in order (Tauri runs async commands concurrently). Commands are `async` (never on the main thread); GPU work runs in
`spawn_blocking`. The shell owns the open documents (one per tab, each with its own history and
view state `Viewport`); every request names its document, and requests for a closed document are
rejected. Frames are returned as
`tauri::ipc::Response` (an `ArrayBuffer` in JS): a 40-byte header (size, fit flag, document
revision, zoom, engine render time) followed by the pixels, parsed without copy in `engine.ts`.

## Key invariants

1. **Engine owns the document.** The UI holds only views (DTOs) and sends intents.
2. **Every document change is an `Edit`**, hence undoable. Initial content built at load time
   may bypass history by applying edits directly to the document.
3. **Ids, not indices**, identify things across time and across the IPC.
4. **Explicit pixel formats and color spaces**; conversions are named functions; clipping only
   happens at the display boundary.
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
- **A document file format**: not designed yet; must store edits/nodes, not just pixels.

## Open questions (hard to change later)

| Topic                         | Current choice                          | Decide before                     |
| ----------------------------- | --------------------------------------- | --------------------------------- |
| Viewport presentation         | Frames over IPC ([ADR 0002](adr/0002-viewport-frame-transport.md)) | interactive tools (brush, pan/zoom at 60 fps) |
| Document model (stack vs DAG) | Stack of layers with stable ids         | first non-trivial node type       |
| Color management              | Named spaces (linear sRGB working)      | first real image import           |
| Tile storage / out-of-core    | Geometry only                           | pixel layers                      |
| HD generative AI strategy     | Research only                           | first AI feature                  |
| File format                   | None                                    | save/load                         |
