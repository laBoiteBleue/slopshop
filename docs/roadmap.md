# Roadmap

A living plan, ordered by dependency rather than by date. Priorities are validated by the
maintainer; each phase ends with a usable, tested state. Items marked 🔶 require a decision
(ADR) before implementation — see the open questions in [architecture.md](architecture.md).

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
- [ ] Import phases 2–6: JPEG XL, JPEG 2000, AVIF; DICOM, FITS; camera RAW; PSD/KRA/XCF/ORA,
      SVG, PDF; optional native backends
- [x] Export (PNG/TIFF/EXR first, [ADR 0008](adr/0008-export.md))
- [x] JPEG and WebP export, background color for alpha-less exports
      ([ADR 0010](adr/0010-jpeg-webp-export.md))
- [x] Gray export: PNG, TIFF, JPEG, the default for gray documents
      ([ADR 0011](adr/0011-gray-export.md))
- [ ] Export: an "encoding" progress phase for WebP; gray EXR (needs a luminance-only reader)
- [ ] Open a folder or a zip archive like a multi-file open: its compatible images become tabs,
      or layers when dropped on the canvas
- [x] Document file format v0 (`.slop`: layers/nodes and tiles, incremental crash-safe saves,
      [ADR 0009](adr/0009-document-file-format.md), [specification](file-format.md)); Save /
      Save As in the app, `slopshop save` / `inspect` in the CLI
- [ ] Document format: a fuzz target for the reader
- [x] Menu bar in Photoshop's order and names, with its shortcuts
      ([ADR 0013](adr/0013-familiar-layout.md)); tools, options bar and more panels come with
      the features that need them
- [ ] Window behavior: a second launch focuses the running window (and opens its files in
      tabs); window size and position remembered
- [ ] Layer thumbnails in the layers panel

## Phase 2 — Non-destructive core

- [x] Blend modes (Photoshop's, except Dissolve) and a blend space per document: perceptual or
      linear ([ADR 0012](adr/0012-blend-modes.md))
- [ ] Layer masks; Dissolve
- [ ] Non-destructive transforms (move, scale, rotate) with quality resampling
- [ ] Adjustment layers as nodes (levels, curves, hue/saturation, exposure)
- [ ] Groups (a `.slop` or PSD imported into a document arrives as a group); 🔶 stack-to-DAG
      evolution of the model
      ([ADR 0003](adr/0003-document-model-edits-history.md))
- [ ] Render caches keyed by (node, region, level, revision); partial recomputation
- [ ] History: memory limits, named entries, history panel

## Phase 3 — Selection and painting

- [ ] Selection model (as masks), marquee/lasso, selection → mask
- [ ] GPU brush engine on tiles (pressure, spacing, hardness), eraser
- [ ] Undo of pixel edits by tile reference (no whole-layer copies)
- [ ] Tool system and options bar

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
