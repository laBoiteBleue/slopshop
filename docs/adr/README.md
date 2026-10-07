# Architecture Decision Records

Short records of significant decisions: context, decision, alternatives, consequences.
Statuses: *proposed* (current direction, open to change), *accepted*, *superseded by NNNN*.

| #    | Title                                                               | Status   |
| ---- | ------------------------------------------------------------------- | -------- |
| 0001 | [Rust engine, Tauri shell, Svelte UI](0001-stack-and-engine-boundary.md) | accepted |
| 0002 | [Viewport presentation](0002-viewport-frame-transport.md)           | accepted direction |
| 0003 | [Document model, edits and history](0003-document-model-edits-history.md) | accepted |
| 0004 | [UI internationalization](0004-ui-internationalization.md)          | accepted |
| 0005 | [Pixel storage v0: in-memory tiles and pyramid](0005-pixel-storage-v0.md) | accepted (v0) |
| 0006 | [Universal import strategy and dependency licensing](0006-universal-import-and-licensing.md) | accepted |
| 0007 | [Color management and working space](0007-color-management.md) | accepted |
| 0008 | [Export](0008-export.md)                                        | accepted |
| 0009 | [Document file format v0](0009-document-file-format.md)         | accepted |
| 0010 | [JPEG and WebP export](0010-jpeg-webp-export.md)                | accepted |
| 0011 | [Gray export](0011-gray-export.md)                              | accepted |
| 0012 | [Blend modes and blend space](0012-blend-modes.md)              | accepted |
| 0013 | [A layout familiar to Photoshop users](0013-familiar-layout.md) | accepted |
| 0014 | [Layer masks](0014-layer-masks.md)                              | accepted |
| 0015 | [Layer groups](0015-layer-groups.md)                            | accepted |
| 0016 | [Clipping masks](0016-clipping-masks.md)                        | accepted |
| 0017 | [Non-destructive transforms](0017-non-destructive-transforms.md) | accepted |
| 0018 | [Resampling transformed layers](0018-resampling.md)             | accepted |
| 0019 | [DXC compiles the shaders on Windows](0019-dxc-shader-compiler.md) | accepted |
| 0020 | [Adjustment layers](0020-adjustment-layers.md)                     | accepted |
| 0021 | [AVIF import with rav1d, without assembly](0021-avif-import.md)    | accepted |
| 0022 | [Display cache: composited tiles addressed by their content](0022-display-cache.md) | accepted |
| 0023 | [Camera RAW through a separate helper process](0023-camera-raw-helper.md) | accepted |
| 0024 | [Selections](0024-selections.md) | accepted |
| 0025 | [AI selection](0025-ai-selection.md) | accepted |
| 0026 | [Quick Selection by color](0026-quick-selection.md) | accepted |
| 0027 | [Painting: brush strokes on tiles](0027-painting.md) | accepted (points 3–5 revised by 0029) |
| 0028 | [Document resolution (pixels per inch)](0028-resolution.md) | accepted |
| 0029 | [A layer's own stack: paint and applied effects](0029-layer-stack.md) | accepted (points 3–5 revised by 0034) |
| 0030 | [The panels dock](0030-panel-dock.md) | accepted |
| 0031 | [Bake to Pixels: Rasterize, Merge, Merge Visible, Flatten](0031-bake-to-pixels.md) | accepted |
| 0032 | [Layer styles: effects drawn from a layer's shape](0032-layer-styles.md) | accepted |
| 0033 | [Project license: GPL-3.0-only](0033-project-license.md) | accepted |
| 0034 | [Editable operations: the stack's entries, filters and filter layers](0034-editable-operations.md) | accepted |
| 0035 | [Filters on the GPU](0035-filters-on-the-gpu.md) | accepted |
| 0036 | [Panels, the Window menu and the saved layout](0036-panels-and-layout.md) | accepted |
| 0037 | [Liquify: a displacement field kept as an entry of the layer's stack](0037-liquify.md) | accepted |
| 0038 | [Projective layer transforms: Distort and Perspective](0038-projective-transforms.md) | accepted |
| 0039 | [In-app updates](0039-in-app-updates.md) | accepted |
| 0040 | [Sources: what layers show, kept once and referenced](0040-sources.md) | accepted |
| 0041 | [Vector content: shapes, paths, text and vector masks](0041-vector-content.md) | accepted |

New ADR: copy the structure of an existing one, next number, add it to this table.
