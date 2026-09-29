# 0005 — Pixel storage v0: in-memory immutable tiles with a display pyramid

Status: accepted for v0 (revisit before out-of-core storage and painting)

## Context

The editor must open real, large images (tens to hundreds of megapixels) without ever assuming
that an image fits in VRAM, keep source pixels intact (non-destructive), and let document
snapshots and undo share pixels instead of copying them.

## Decision

- `slopshop-core::raster::RasterImage`: an **immutable** image stored as fixed **256 × 256
  tiles** in its **source pixel format**, each tile an `Arc<[u8]>`. Edge tiles are padded by
  repeating the last row/column so every tile has the same size.
- A **display pyramid** (levels halved down to one tile) is built at import time. It is derived
  data, averaged in **linear light with premultiplied alpha**; the source level is never
  modified.
- Layers hold `LayerContent::Raster { image: Arc<RasterImage> }`: cloning a document is cheap
  and never copies pixels. Every image has a process-unique `ImageId` (cache key).
- Formats: **gray, gray+alpha and RGBA in 8/16-bit integer or 16/32-bit float**, straight or
  premultiplied alpha, with an explicit color space (ADR 0007). RGB without alpha is stored as
  RGBA with an opaque alpha (lossless; GPUs have no 3-channel formats). Nothing else is
  converted. (Revised 2026-09-29: v0 only accepted 8-bit sRGB RGBA.)
- File decoding lives in a separate crate, `slopshop-io` (depends on the `image` crate with
  only the PNG and JPEG decoders), so `slopshop-core` stays dependency-free.
- Rendering: the GPU keeps a **tile cache per storage class** (RGBA8, RGBA16 integer, RGBA16
  float, RGBA32 float texture arrays within a VRAM budget, LRU eviction); values are uploaded as
  stored and decoded/converted in the shader. Each frame picks the pyramid level matching the zoom, uploads only the visible
  tiles that are missing, and falls back to a coarser level if they would not fit.

## Alternatives

- **One big texture per layer**: simplest, but bounded by GPU texture limits (typically 16K)
  and VRAM; incompatible with 100+ MP documents.
- **Store tiles in the working format (linear f16/f32)**: uniform compositing input, but 2–4×
  the memory and a lossy or bloating conversion at import. Keeping the source format and
  converting on the GPU at sampling time is lossless and compact.
- **Out-of-core tiles now** (disk-backed, memory-mapped): required eventually (Phase 4), but
  premature before the tile API has settled. The tile API is designed so that residency can
  become lazy without changing callers.

## Consequences

- RAM ≈ 1.33 × the RGBA8 size of the image for typical aspect ratios (e.g. ~1.2 GB for 233 MP;
  edge-tile padding inflates very thin strips), plus a transient decode buffer at import.
  Images larger than RAM are not supported yet.
- Imports are checked from the file header before decoding: non-8-bit formats are rejected and
  the estimated memory must stay under a fixed budget (32 GiB), so a corrupt or crafted header
  cannot abort the process. A budget derived from the machine's RAM comes with out-of-core
  storage.
- The GPU tile budget is planned for all raster layers of a frame together: layers get coarser
  levels rather than disappearing, and layers sharing an image share its tiles.
- Sampling is nearest-neighbour at the chosen level: pixel-exact at 100%, slightly aliased when
  zoomed out between levels. Filtered sampling (bilinear/trilinear with tile gutters) is future
  work.
- Gray tiles are expanded to RGBA when uploaded to the GPU (more VRAM for gray images); a
  single-channel path can come with DICOM/FITS.
- Painting will need mutable tiles: copy-on-write per tile (`Arc::make_mut`-style) so undo keeps
  the old tiles by reference.
