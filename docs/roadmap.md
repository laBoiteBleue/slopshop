# Roadmap

A living plan, ordered by dependency rather than by date. Priorities are validated by the
maintainer; each phase ends with a usable, tested state. Items marked 🔶 require a decision
(ADR) before implementation — see the open questions in [architecture.md](architecture.md).
Ergonomics (principles, ideas waiting for validation) have their own page:
[ergonomics.md](ergonomics.md).

## Phase 0 — Foundations *(in progress)*

- [x] Cargo workspace, lints, toolchain pinning
- [x] Core: document model, reversible edits, undo/redo, gestures, tiling geometry, color types
- [x] Headless wgpu renderer (view → frame), CPU-reference GPU tests
- [x] `slopshop` CLI (GPU info, headless render)
- [x] Desktop shell: viewport, layers panel (visibility, live opacity, rename, reorder,
      delete, add fill), undo/redo, EN/FR interface
- [x] Docs: README, CLAUDE.md, architecture, ADRs, HD-AI research notes
- [ ] CI green on Linux, Windows and macOS (first push)

## Phase 1 — Real images, real viewport

- [x] Pan and zoom (view state owned by the engine; fit, 100%, preset steps, Ctrl+wheel zoom
      around the cursor, wheel / Space+drag / middle-drag pan), frame timing in the status bar
- [x] Viewport presentation direction: native GPU surface, frames as fallback
      ([ADR 0002](adr/0002-viewport-frame-transport.md))
- [x] Native surface on Windows: the engine presents under a transparent webview (ADR 0002)
- [ ] Native surface on macOS (Metal view; needs a Mac)
- [x] Multi-document tabs; open into a new tab; drop on the canvas adds a layer, drop elsewhere
      opens new tabs; Ctrl+N / Ctrl+W / Ctrl+Tab
- [x] Universal import strategy decided ([ADR 0006](adr/0006-universal-import-and-licensing.md),
      [research](research/universal-import.md))
- [x] Tile-backed pixel layers in core (immutable shared tiles, [ADR 0005](adr/0005-pixel-storage-v0.md))
- [x] GPU tile upload/cache; render only visible tiles
- [x] Mip levels for zoomed-out views, area-filtered in linear light
- [x] Smooth navigation: animated wheel zoom, instant reprojection of the last frame
- [x] Color management model and working space ([ADR 0007](adr/0007-color-management.md))
- [x] Open 8-bit PNG/JPEG (dialog, drag and drop, command line; dev builds open a test image)
- [x] Engine: 8/16-bit and 16/32-bit float storage, gray and alpha, color-managed compositing
      in linear Rec.2020 ([ADR 0007](adr/0007-color-management.md))
- [x] Import (phase 1 of [ADR 0006](adr/0006-universal-import-and-licensing.md)): PNG (16-bit),
      JPEG, TIFF (8/16/32-bit float), WebP, GIF, BMP, TGA, ICO, PNM/PFM, QOI, farbfeld, EXR, HDR,
      DDS; matrix/TRC ICC profiles; EXIF orientation; license policy enforced by `cargo-deny`
- [x] PSD/PSB import, stage 1: the flattened composite (8/16/32-bit, bitmap, gray, indexed,
      RGB, duotone; transparency, ICC; raw, RLE, zip)
- [x] PSD/PSB import, stage 2: layers with their blend modes, opacity, visibility and masks,
      solid color fills; what the engine lacks is reported layer by layer
- [x] PSD/PSB import: groups as groups (pass-through or isolated, with their masks)
- [x] PSD/PSB import: clipping masks
- [x] PSD export (layered, 8/16-bit): layers, groups, clipping, masks, blend modes and
      adjustment layers, with a merged composite ([formats plan](formats.md#psd-and-psb-p0))
- [x] PSB export (above 30,000 px per side), streamed: memory does not grow with the document
- [x] JPEG XL import (jxl-oxide)
- [x] AVIF import (rav1d without assembly, [ADR 0021](adr/0021-avif-import.md))
- [x] AVIF export (rav1e without assembly): 8/10-bit, alpha, gray, HDR spaces
- [x] JPEG XL export, lossless (zune-jpegxl): 8/16-bit, alpha, gray, every space
- [ ] JPEG XL lossy export (needs libjxl, C++)
- [x] JPEG 2000 import (hayro-jpeg2000): JP2 and raw codestreams, native depth
- [x] DICOM import (dicom-rs): native precision, every slice of a file or a series in one
      isolated group under one window (a Levels layer)
- [x] FITS import (in-house): native precision, an automatic stretch as a Levels layer
- [x] SVG import (resvg), through the Import PDF dialog made generic (Import SVG)
- [x] Export to QOI, farbfeld, Radiance HDR, ICO, GIF (one frame, 256 colors) and DDS
      (uncompressed); uncompressed DDS import
- [x] Export to DICOM (Secondary Capture), FITS and PDF (one page)
- [x] Export to JPEG 2000 (openjp2): JP2, lossless or lossy; with the items above, an export
      for every format read (not camera RAW, not SVG)
- [x] Camera RAW, basic: developed "as shot" to linear Rec.2020 in a separate helper process,
      rawler being LGPL ([ADR 0023](adr/0023-camera-raw-helper.md))
- [ ] Other formats (KRA/XCF/ORA, legacy formats, a RAW development module): the format track
      closes after the items above (maintainer, 2026-10-01); open to contributors, with each
      format's priority and approach in [formats.md](formats.md)
- [x] PDF import: the Import PDF dialog (pages as thumbnails, resolution or size), pages
      rasterized as 8-bit sRGB, the picked pages in one group over a white background
      ([formats plan](formats.md#pdf-p1))
- [x] Export (PNG/TIFF/EXR first, [ADR 0008](adr/0008-export.md))
- [x] JPEG and WebP export, background color for alpha-less exports
      ([ADR 0010](adr/0010-jpeg-webp-export.md))
- [x] Gray export: PNG, TIFF, JPEG, the default for gray documents
      ([ADR 0011](adr/0011-gray-export.md))
- [ ] Export: an "encoding" progress phase for WebP; gray EXR (needs a luminance-only reader)
- [x] Open a folder (File > Open Folder, or dropped) or a zip archive like a multi-file open:
      its images and documents, in natural order, become tabs, or layers when dropped on the
      canvas or the layers panel
- [x] Document file format v0 (`.slop`: layers/nodes and tiles, incremental crash-safe saves,
      [ADR 0009](adr/0009-document-file-format.md), [specification](file-format.md)); Save /
      Save As in the app, `slopshop save` / `inspect` in the CLI
- [ ] Document format: a fuzz target for the reader
- [x] Menu bar in Photoshop's order and names, with its shortcuts
      ([ADR 0013](adr/0013-familiar-layout.md)); tools, options bar and more panels come with
      the features that need them
- [x] Window behavior: a second launch focuses the running window (and opens its files in
      tabs); window size, position and maximized state remembered
- [x] Layer thumbnails in the layers panel
- [x] Drag layers onto another tab (it shows after a short hover), drop them on its image or
      layers panel to copy them there
- [x] Right-click menu on layers (the Layer menu's commands), Duplicate Layer (Ctrl+J), Hide or
      Show Layers
- [x] Layer multi-selection (Ctrl/Shift+click, Select > All Layers): delete, move, opacity and
      blend mode apply to every selected layer, as one undo entry
- [x] Paste (Ctrl+V): a copied image becomes a layer (or a new tab), copied files open like
      dropped ones
- [x] Copy, Cut and Paste layers (Ctrl+C, Ctrl+X, Ctrl+V): on top of any document, or as a new
      one (Paste as New Document); the pixels are shared, so copying costs nothing
- [ ] Drop images from a web page (browsers drag URLs or virtual files, not file paths)

## Phase 2 — Non-destructive core

- [x] Blend modes (Photoshop's) and a blend space per document: perceptual or
      linear ([ADR 0012](adr/0012-blend-modes.md))
- [x] Layer masks: from a layer's transparency, enabled or disabled, deleted
      ([ADR 0014](adr/0014-layer-masks.md)); thumbnails in the layers panel
- [x] Dissolve blend mode (a noise fixed by document position, identical on CPU and GPU)
- [ ] Painting in masks (with the brushes of Phase 3)
- [x] Non-destructive transforms, step 1 ([ADR 0017](adr/0017-non-destructive-transforms.md)): a
      transform per layer (a group's applies inside it), whole-pixel moves rendered exactly,
      the Move tool (drag on the image, arrows, Shift+arrows), `.slop` 0.6
- [x] Move tool: Auto-Select (the layer under the pointer), snapping to the canvas and to the
      other layers (edges and centers) with smart guides, View > Snap
- [x] Transforms, step 2a ([ADR 0018](adr/0018-resampling.md)): any invertible transform
      rendered with quality resampling (EWA Lanczos sharp with anti-ringing, identical on the
      CPU and the GPU; quarter turns and flips stay exact)
- [x] Transforms, step 2b: Free Transform (Ctrl+T: move, scale, rotate), Edit > Transform
      (quarter turns and flips, exact)
- [x] Transforms, step 2c: Image Size, Canvas Size and Image Rotation as transforms of the
      layers (nothing cut, nothing rewritten)
- [x] Crop tool (C): a frame on the image with handles, snapping, Enter applies; nothing is
      deleted
- [ ] A detail option for enlargements (Photoshop's Preserve Details crispness): a detail pass
      after resampling, clamped to the local range (see the comparison of 2026-09-30)
- [ ] Image > Crop to the selection, with selections
- [x] Adjustment layers, engine ([ADR 0020](adr/0020-adjustment-layers.md)): Exposure,
      Hue/Saturation and Levels, applied to what is below (opacity, mask, clipping, groups),
      CPU and GPU, `.slop` 0.7
- [x] Adjustment layers, UI: Layer > New Adjustment Layer, a Properties panel with live sliders
- [x] Brightness/Contrast, Vibrance, Invert, Posterize, Threshold
- [x] PSD import of adjustment layers: the eight reproduced ones become adjustment layers
- [x] Black & White, Color Balance, Photo Filter, Channel Mixer (up to 16 parameters, `.slop` 0.8)
- [x] PSD import and export of these four
- [x] Curves (with a curve editor), `.slop` 0.9, PSD import and export
- [x] Groups in the engine ([ADR 0015](adr/0015-layer-groups.md)): a layer tree, pass-through
      and isolated groups with opacity, blend mode and mask, one-pass CPU and GPU compositing,
      `.slop` 0.4
- [x] Clipping masks in the engine ([ADR 0016](adr/0016-clipping-masks.md)): clipped layers blend
      atop their base, whose mode and opacity apply to the clipping group; `.slop` 0.5
- [x] Clipping masks in the layers panel (Alt+Ctrl+G, Alt+click between layers, indented
      arrow, underlined base)
- [x] Groups in the layers panel: folders that fold, New Group, Group Layers (Ctrl+G), Ungroup
      Layers (Shift+Ctrl+G), drag into and out of groups, Pass Through in the blend modes
- [x] PSD groups; a document of several layers (`.slop`, PSD, a tab) imported into another
      arrives as a group named after it
- [ ] 🔶 Stack-to-DAG evolution of the model
      ([ADR 0003](adr/0003-document-model-edits-history.md))
- [ ] Render caches keyed by (node, region, level, revision); partial recomputation
- [ ] History: memory limits, named entries, history panel

## Phase 3 — Selection and painting

Order (maintainer, 2026-10-01): the tools, then selections, then painting, so that the brush
respects the selection from the start. Painting follows Photoshop's model: a stroke rewrites the
layer's tiles copy-on-write and undo keeps the old tiles by reference; non-destructive work
paints on an empty layer or in a mask.

- [x] Toolbar and options bar ([ADR 0013](adr/0013-familiar-layout.md)): the Move, Crop, Hand
      and Zoom tools with Photoshop's keys; the other tools come with their features
- [ ] Selection model (as masks), marquee/lasso, selection → mask
- [ ] GPU brush engine on tiles (pressure, spacing, hardness), eraser
- [ ] Undo of pixel edits by tile reference (no whole-layer copies)

## Phase 4 — Very large images

- [ ] Out-of-core tile store (disk cache), RAM/VRAM budgets
- [ ] Background computation with progress and cancellation
- [ ] Benchmarks on 100–500 MP documents, tracked in CI

## AI track *(research runs in parallel from Phase 1)*

- [ ] Benchmark set and experiment harness
      ([research notes](research/hd-generative-ai.md))
- [ ] Compare HD strategies (baseline, ROI, coarse-to-fine, tiled diffusion, HF re-injection)
- [ ] 🔶 Inference runtime inside the app (ONNX Runtime, candle, burn, sidecar)
- [ ] 🔶 Chosen HD pipeline, recorded with evidence
- [ ] AI node model: prompt, model + version, seed, mask, ROI, dependencies, cached result,
      stale/invalidated state, preview vs. full-definition render
- [ ] First AI feature (likely object removal / generative fill in a region), optional and
      local

## Phase 5 — Extensibility and distribution

- [ ] 🔶 Plugin/extension API (nodes, importers/exporters)
- [ ] Scripting and batch processing through the CLI
- [ ] Signed releases, installers, auto-update
- [ ] More interface languages (one catalog each)
