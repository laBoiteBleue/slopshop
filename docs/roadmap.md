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

- [ ] Pan and zoom (view state owned by the engine; 100% zoom, fit, wheel zoom around cursor)
- [ ] 🔶 Viewport presentation path: measure frame cost, decide between frames over IPC and a
      native wgpu surface ([ADR 0002](adr/0002-viewport-frame-transport.md))
- [ ] Tile-backed pixel layers in core (copy-on-write tiles shared by snapshots and history)
- [ ] GPU tile upload/cache; render only visible tiles
- [ ] Mip levels for zoomed-out views
- [ ] 🔶 Color management scope (ICC profiles in/out, OCIO later?)
- [ ] Import PNG/JPEG/TIFF (8/16-bit) with explicit conversion into the working space;
      export PNG/TIFF
- [ ] 🔶 Document file format v0 (stores layers/nodes and tiles, not a flattened image)

## Phase 2 — Non-destructive core

- [ ] Layer masks and blend modes
- [ ] Non-destructive transforms (move, scale, rotate) with quality resampling
- [ ] Adjustment layers as nodes (levels, curves, hue/saturation, exposure)
- [ ] Groups; 🔶 stack-to-DAG evolution of the model
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
