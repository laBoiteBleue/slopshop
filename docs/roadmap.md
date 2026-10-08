# Roadmap

A living plan, ordered by dependency rather than by date. Priorities are validated by the
maintainer; each phase ends with a usable, tested state. Items marked 🔶 require a decision
(ADR) before implementation — see the open questions in [architecture.md](architecture.md).
Ergonomics (principles, ideas waiting for validation) have their own page:
[ergonomics.md](ergonomics.md). What SlopShop does with each feature of professional editors,
including the ones it leaves out: [feature-map.md](feature-map.md).

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
      one; the pixels are shared, so copying costs nothing
- [x] Clipboard as in Photoshop: Copy and Cut of the selected pixels, Copy Merged, Paste in
      Place, Paste Into (a group masked by the selection), pastes placed where they were copied
      or in the view, an 8-bit image for other applications, File > New's Clipboard preset
- [x] File > New, as Photoshop's New dialog: a name, presets, the size and its orientation, the
      background contents (white, black, the background color, transparent)
- [ ] Drop images from a web page (browsers drag URLs or virtual files, not file paths)

## Phase 2 — Non-destructive core

- [x] Blend modes (Photoshop's) and a blend space per document: perceptual or
      linear ([ADR 0012](adr/0012-blend-modes.md))
- [x] Layer masks: from a layer's transparency, enabled or disabled, deleted
      ([ADR 0014](adr/0014-layer-masks.md)); thumbnails in the layers panel
- [x] Dissolve blend mode (a noise fixed by document position, identical on CPU and GPU)
- [x] Painting in masks and in Quick Mask (ADR 0027): a click on a mask's thumbnail makes it the
      target (new masks are), the Brush paints the gray of its color, the Eraser hides
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
- [x] View menu (decided 2026-10-04, docs/ergonomics.md): Hide Extras (Ctrl+H), Snap
      remembered, Full Screen (F11), Window > Hide Panels (Tab)
- [x] Rulers (Ctrl+R) and guides in the document (drag from a ruler, snapping, Clear Guides,
      `.slop` 0.22), a pixel grid at high zoom
- [x] Free Transform's upright edges on whole pixels; ruler units (a right-click on a ruler)
- [x] Image > Image Rotation > Arbitrary: the layers turned by any angle through their
      transforms, the canvas grown to hold them (Photoshop's Rotate Canvas)
- [x] Image > Trim (transparent or corner-colored margins, composited in bands from the edges)
      and Reveal All (the canvas grown to every layer's pixels), both crops
- [ ] Image > Auto Tone, Auto Contrast, Auto Color (Levels per channel first)
- [ ] Gradient Map and Selective Color adjustments
- Dropped (2026-10-02): a detail option for enlargements (Photoshop's Preserve Details
  crispness). A sharpening kernel folded into the resampling filter, clamped to the local range,
  gained about 2% of error against originals reduced 4× and enlarged back: edges are crisper,
  no detail comes back, and textures turn patchy at full strength. Enlargements that recover
  detail belong to the AI track (AI upscaling).
- [x] Adjustment layers, engine ([ADR 0020](adr/0020-adjustment-layers.md)): Exposure,
      Hue/Saturation and Levels, applied to what is below (opacity, mask, clipping, groups),
      CPU and GPU, `.slop` 0.7
- [x] Adjustment layers, UI: Layer > New Adjustment Layer, a Properties panel with live sliders
- [x] Brightness/Contrast, Vibrance, Invert, Posterize, Threshold
- [x] PSD import of adjustment layers: the eight reproduced ones become adjustment layers
- [x] Black & White, Color Balance, Photo Filter, Channel Mixer (up to 16 parameters, `.slop` 0.8)
- [x] PSD import and export of these four
- [x] Curves (with a curve editor), `.slop` 0.9, PSD import and export
- [x] Levels per channel (Photoshop's Channel menu), `.slop` 0.13, PSD import and export
- [x] Image > Auto Tone, Auto Contrast, Auto Color: Levels computed from the visible image
- [x] Gradient Map (a simple gradient editor), `.slop` 0.14, PSD import and export
- [x] Selective Color (Bœsch's measured model of Photoshop's), `.slop` 0.15, PSD import and
      export
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
- [x] Layer menu restructured (New, fill and adjustment layers, Arrange with Ctrl+[ and ],
      commands on every selected layer), after an audit against Photoshop's
      ([ergonomics](ergonomics.md#layer-menu-audit-of-2026-10-03))
- [x] Solid color fill layers: the foreground color, above the active layer, editable after
      creation (Properties panel, double-click on the thumbnail)
- [x] Layer > Align and Distribute, shared with the Move tool's options bar
- [x] New Layer from Visible (stamp visible, Alt+Shift+Ctrl+E)
- [x] Layer > Bake to Pixels ([ADR 0031](adr/0031-bake-to-pixels.md)): Rasterize, Merge
      Layers and Merge Down (Ctrl+E), Merge Visible (Shift+Ctrl+E), Flatten Image; merges
      and New Layer from Visible show at once, their pixels following in the same undo entry
- [x] Layer styles in the engine ([ADR 0032](adr/0032-layer-styles.md)): Drop Shadow, Stroke and
      Color Overlay drawn from a layer's shape with the selection's coverage operations, Fill
      Opacity, composited around the content (CPU and GPU alike), drawn again when the layer
      changes
- [x] Layer styles in `.slop` (node v8) and through the IPC
- [x] The Layer Style dialog, Layer > Layer Style, the fx mark and the effects listed below
      the layer with their eyes, Fill in the layers panel
- [x] Layer styles: Outer and Inner Glow, Inner Shadow (`.slop` node v9)
- [x] Layer styles: PSD import and export (`lfx2`, Fill), styles on groups (drawn from
      what they hold)
- [x] Layer styles: Gradient Overlay (Linear and Radial, angle, scale, reverse, aligned with
      the layer or the canvas; drawn as a gradient fill within the shape; PSD `GrFl`), `.slop`
      node v12
- [x] Layer styles: Satin (the blurred shape against itself moved, inverted or not; a linear
      contour; PSD `ChFX`), `.slop` node v13
- [x] Layer styles: Bevel and Emboss (Inner, Outer, Emboss, Pillow; Smooth; depth, direction,
      size, soften, light angle and altitude, highlight and shadow; PSD `ebbl`), `.slop` node v14
- [ ] Stack-to-DAG evolution of the model
      ([ADR 0003](adr/0003-document-model-edits-history.md)): settled by
      [ADR 0040](adr/0040-sources.md), a tree of layers referencing read-only sources
- [ ] Render caches keyed by (node, region, level, revision); partial recomputation
- [x] History panel and named history entries ([ADR 0036](adr/0036-panels-and-layout.md))
- [x] Histogram and Info panels ([ADR 0036](adr/0036-panels-and-layout.md))
- [ ] History: memory limits

## Phase 3 — Selection and painting

Order (maintainer, 2026-10-01): the tools, then selections, then painting, so that the brush
respects the selection from the start. Painting ([ADR 0027](adr/0027-painting.md)) never writes
a layer's original pixels or mask. A layer's paint and applied effects form a stack inside it
([ADR 0029](adr/0029-layer-stack.md)): entries removable one by one, and editable since
[ADR 0034](adr/0034-editable-operations.md); a mask's
paint is a painted image sharing the untouched tiles, removable as a whole.

- [x] Toolbar and options bar ([ADR 0013](adr/0013-familiar-layout.md)): the Move and Crop
      tools with Photoshop's keys (no Hand or Zoom tool: the wheel, Space and the middle button
      do that); the other tools come with their features
- [x] Selection model in the engine ([ADR 0024](adr/0024-selections.md)): 16-bit coverage masks
      sharing their uniform tiles; rectangles, ellipses and polygons rasterized exactly;
      replace, add, subtract, intersect; feather; inverse; outline for the marching ants;
      undoable
- [x] Rectangular and Elliptical Marquee, their options (modes, feather, anti-alias), marching
      ants, the Select menu (All, Deselect, Reselect, Inverse), tool groups in the toolbar
- [x] Quick Mask (Q): a view overlay drawn by the GPU after compositing, never cached
- [x] Marching ants drawn by the GPU in the natively presented view (ADR 0024, amended): the
      selection's coverage sampled per frame pixel, so they stay in step with the image while
      scrolling a 233 MP image, with no CPU trace or IPC round trip; the SVG ants remain for
      frames over IPC (macOS, Linux: to test there)
- [x] Lasso and Polygonal Lasso
- [x] Selection → layer mask (Reveal / Hide Selection, and Reveal / Hide All), Image > Crop to
      the selection
- [x] Select > Modify: Border, Smooth, Expand, Contract (exact distance to the outline, rounded
      corners), Feather (Shift+F6)
- [x] Magic Wand (W): tolerance, contiguous or not, anti-alias, the active layer or every layer;
      tile by tile with a bounded cache (50 MP: about 1 s with a few layers on the CPU)
- [x] The selection tools (Magic Wand, Grow, Similar, Color Range) read their pixels from the GPU
      (the export path, rows of tiles, the CPU compositor as fallback and reference): 50 MP of 9
      layers 5 to 11 times faster, identical selections on flat colors
      ([ADR 0024](adr/0024-selections.md#amendment-2026-10-04-the-colors-come-from-the-gpu))
- [x] Select > Color Range: eyedroppers (sample, add, subtract) on the image or a live
      preview, Fuzziness, Invert, within the selection if any
- [x] AI runtime ([ADR 0025](adr/0025-ai-selection.md)): the `slopshop-ai` helper (ONNX
      Runtime loaded at run time), components downloaded with consent (Edit > Preferences)
- [x] Object Selection (W): the object under the pointer lights up, a click or a box selects
      it (SAM 2.1, coarse masks)
- [x] Quick Selection (W) by color, as Photoshop's ([ADR 0026](adr/0026-quick-selection.md)):
      the region of similar colors around the stroke, up to the image's edges; Alt subtracts
- [x] Refine Edges at full resolution (ViTMatte on windows along the outline): an option of the
      AI tools, and Select and Mask's edge detection for any selection
- [x] Select > Subject (BiRefNet; the lite model on the processor), refined like the tools
- [x] Select menu refactor ([ergonomics](ergonomics.md#select-menu-audit-of-2026-10-04)):
      Photoshop's order, Select Subject, Select and Mask…, Quick Mask Mode; Grow and Similar on
      the Magic Wand's engine
- [x] Transform Selection on Free Transform's handles (the selection resampled as a layer is)
- [x] Color Range: Localized (each sampled color near its sample), Sample All Layers
- [x] Live preview of Select > Modify (a gesture replaced at each amount)
- [x] Quick Mask feedback: its label, its own gray swatches as Add / Remove, overlay opacity
- [x] Saved selections: named objects in the document and `.slop` (schema 0.16), Save / Load
      Selection, transformed with the canvas
- [x] Right panels: the folding dock of tab icons below Layers ([ADR 0030](adr/0030-panel-dock.md)),
      a Selections panel (load, add, subtract, intersect, replace, rename, delete), Window menu
- [x] Select and Mask… panel: edge detection, Smooth, Feather, Contrast, Shift Edge, views,
      outputs
- [x] Select and Mask: the Refine Edge Brush (the model also decides where it paints)
- Dropped (2026-10-02): Select > Semantic (SAM 3), objects named by a text prompt. A click with
  Object Selection is simpler.
- [x] Painting decided ([ADR 0027](adr/0027-painting.md)): strokes on the CPU as a coverage,
      shown by the GPU; paint kept apart from the original, Delete Paint
- [x] Brush engine in the core (diameter, hardness, spacing, flow, opacity, pressure), the
      painted image and its undo by tile reference, a benchmark
- [x] Tiles keyed by identity in the GPU cache and the `.slop` writer; painted images in `.slop`
- [x] Brush (B) and Eraser (E) in the app: options bar, colors and picker, pen pressure, brush
      outline, new empty layer (Shift+Ctrl+N), Delete Paint and the painted-layer mark
- [x] Delete with a selection erases the selected pixels (Photoshop's Clear), as paint
- [x] Edit > Fill (Shift+F5): the foreground or background color, a color picked, black, 50%
      gray or white, at an opacity, in the selection or the whole layer; Edit > Stroke: a band
      inside, centered on or outside the selection's outline, its width, color and opacity; both
      as paint (not editable afterwards, ADR 0029)
- [x] Move tool inside a selection, as Photoshop: the selected pixels of the active layer (or of
      its targeted mask) move with the selection, leaving a hole; Alt copies them; arrows nudge
      them; they float until something else happens; pixels moved off the canvas are kept (the
      layer grows); kept as paint (the original intact)
- [x] Selected pixels float in the view during the drag (an isolated group of the layer with
      its hole and the moved pixels), the move computed once on release
- [x] Move the selection's outline alone, as Photoshop: a drag from inside it with the marquees,
      the Lasso or the Magic Wand (New Selection mode), the arrows with any selection tool
- [x] Layer via Copy (Ctrl+J) and Layer via Cut (Shift+Ctrl+J) with a selection, as Photoshop
- [x] A layer's own stack in the engine ([ADR 0029](adr/0029-layer-stack.md)): paint as a delta
      (`P + k·B`, touched tiles only), effects as parameters with their selection, merges of
      alike neighbours, evaluation on the CPU tile by tile, each change reevaluating only the
      tiles it reaches
- [x] Layers carry their stack: strokes, Fill, Stroke, Delete and moved pixels add paint on
      top of it (what is below the paint cached for the stroke), `.slop` node v7 (v6 paint read
      exactly), Delete Paint empties it
- [x] A layer's stack listed below it in the layers panel (an arrow unfolds it, newest on
      top), entries deleted one by one
- [x] Image > Adjustments as effects on every visible pixel layer, with Photoshop's shortcuts, a
      dialog previewed on the canvas
- [x] The Restore Eraser (in the Eraser's group)
- [x] The stack evaluated by the shader while the layer's pixels are evaluated on a thread of
      their own (ADR 0029, point 6): applying, deleting and undoing are instant
- [x] Frames over the IPC (macOS, Linux) rendered again until a stack's pixels are evaluated,
      as native presents (Windows) are
- [x] Editable operations ([ADR 0034](adr/0034-editable-operations.md)): entries of a stack
      edited again (an icon reopens their dialog), an eye per entry,
      `.slop` 0.20
- [ ] An applied adjustment's selection loaded as the selection or replaced (moved pixels
      stay baked: no replay, ADR 0034 amended on 2026-10-05)
- [x] Filter menu: Gaussian Blur as a stack entry (its result cached, the entries above start
      from it; computed on the CPU, the display showing the layer's previous pixels meanwhile),
      Repeat (Ctrl+F) and Last Filter Settings (Alt+Ctrl+F), `.slop` 0.21
- [x] Filters on the GPU (Gaussian Blur first): live previews of large layers (ADR 0035)
- [x] Sharpen > Unsharp Mask and Other > High Pass, made from the Gaussian blur (CPU and GPU)
- [x] Blur > Motion Blur (a line of samples; beyond 256 pixels on the layer reduced; CPU and GPU)
- [x] Noise > Add Noise (uniform or Gaussian, monochromatic; a grain of the document pixel and
      a seed drawn each time it is applied; CPU and GPU)
- [x] Noise > Dust & Scratches (an exact median of a square, on the CPU; beyond a radius of 8 on
      the layer reduced)
- [x] Sharpen > Clarity and Texture (Lightroom's -100 to 100: pushed from a fine and a broad
      blur, Clarity in the midtones; on the CPU)
- [x] Dust & Scratches, Clarity and Texture on the GPU (looks)
- [x] Blur > Box Blur (beyond a radius of 64 on the layer reduced), Noise > Median (CPU and GPU),
      Other > Maximum and Minimum (Squareness, exact whatever the radius); on the CPU but Median
      and Box Blur (its looks on the GPU up to a radius of 64)
- [x] Stylize > Find Edges, Emboss and Solarize (Find Edges and Solarize applied at once, no
      dialog), Pixelate > Mosaic (cells from the layer's origin, whole across tiles); on the CPU
- [x] Filters that move pixels (ADR 0034 point 7, "the whole image" and "a frame"): each pixel
      the layer sampled where the filter says, around the selection's box or the layer; a
      filter's frame (where a look's crop lies on its layer) given to every filter, so that
      Mosaic's looks align with the layer too: Distort > Pinch, Polar Coordinates, Spherize
      (Normal), Twirl, Other > Offset; on the CPU
- [x] Maximum and Minimum, Find Edges, Emboss, Solarize and Mosaic on the GPU (looks); then the
      distortions (Twirl, Pinch, Spherize, Polar Coordinates, Offset; Offset's transparent edge
      on a layer without transparency stays on the CPU)
- [x] Filter > Liquify (Shift+Ctrl+X, [ADR 0037](adr/0037-liquify.md)): a displacement field kept
      as a stack entry (sparse tiles of a grid of 1, 2 or 4 pixels a node), edited again in its
      own workspace (Forward Warp, Reconstruct, Smooth, Twirl, Pucker, Bloat, Push Left, Freeze
      and Thaw Mask; Size, Density, Pressure, Rate; Restore All), `.slop` 0.23; on the CPU. Not
      yet: Face-Aware, saved meshes, mesh display, pen pressure, a GPU warp
- [ ] Filter layers, after a multi-pass compositor (an ADR); new adjustment, fill and filter
      layers masked by the selection
- [ ] Toolbar audit of 2026-10-05 ([ergonomics](ergonomics.md#toolbar-audit-of-2026-10-05)):
  - [x] Layers chosen in the image with the Move tool (click, Shift+click, right-click list,
        rectangle), the active layer shown in the panel; Ctrl+click on a thumbnail loads it as
        the selection
  - [x] Quick Mask's toolbar button; the Eyedropper (I, Alt with the Brush); Crop's ratio,
        size and Straighten
  - [x] Paint Bucket and Gradient (G); Clone Stamp (S); Healing Brush (J); the Pencil as an
        option of the Brush and the Eraser
  - [ ] Patch (a mode of the Healing Brush), Ctrl+Space zooming, Dodge and Burn (O), Blur,
        Sharpen and Smudge: built, merged one by one
  - [x] Gradient fill layers (Layer > New Fill Layer > Gradient), the Gradient tool's engine;
        style, angle, scale and Reverse in the Properties panel, `.slop` 0.24
  - Moved to the projects below: Remove (J, an AI tool), Distort and Perspective in Free
    Transform (transforms). Later (the audit's choice): Mixer Brush, Color Sampler, Custom
    Shape; Pen, Type and Shapes wait for vector content

## Opened to contributors *(2026-10-06)*

The maintainer opened SlopShop to contributors on 2026-10-06, before the AI and monetization
projects that had been planned first: contribution guide, DCO sign-off (ADR 0033), installers
for Windows, macOS and Linux, a bilingual README with screenshots, the
[feature map](feature-map.md). Development continues with the projects below, in this order
(the maintainer's order of 2026-10-05):

- [x] **Transforms**: Distort and Perspective in Free Transform, the layer's transform made
      projective (ADR 0038: CPU and GPU, `.slop` 0.25, Edit > Transform's entries; painting,
      fills and masks through the map). The rest is left to contributors (maintainer,
      2026-10-05): see "Open to contributors: advanced transforms" below
- [ ] **AI** (the large one): Remove (J) and generative fill, on the AI track below (its ADR
      before the first feature, local models first). Generative fill of a selection is done:
      Delete's choice, FLUX.2 [klein] with erase_v1 on our own ONNX graphs (ADR 0045,
      Windows)
- [ ] 🔶 **Cloud monetization**: what is offered, accounts, billing (to define; an ADR)

Raised on 2026-10-05, to schedule:

- [ ] **Sources** ([ADR 0040](adr/0040-sources.md), 2026-10-07): layers reference read-only
      sources kept once, Duplicate shares them, Make Unique, Replace Contents, a Sources
      panel; this settles the layer stack or DAG question (a tree of layers plus references)
  - [x] The engine: image sources (imports named after their file, pasted and baked pixels
        unnamed, empty layers without), Duplicate sharing them, Rasterize and merges making
        new ones, layers growing around theirs; `.slop` schema 0.26, older files read with a
        source per original
  - [x] Make Unique (Layer > Make Source Unique) and the Sources panel (select the layers
        showing a source, a new layer from it)
  - [x] Linked copies (Duplicate, shapes changed on every copy, a link mark) and Duplicate as
        Independent Copy (ADR 0040 amendment)
  - [x] Bin entries named (their own name, else their first layer's; independent copies
        "… copy"); deleting a source and the layers showing it, after a confirmation
  - [x] Shapes in the bin (a thumbnail, a new linked layer from one, deleted with their layers)
  - [ ] A source dragged onto the canvas, a source renamed
  - [ ] Replace Contents
- [ ] 🔶 Vector content (Pen, Type, Shapes, vector masks): one model, an ADR, as a source
      kind without a stack (ADR 0040, point 8); an editor without the Type tool surprises
      users and contributors alike
  - [x] Vector layers (ADR 0041): shapes as sources, drawn anti-aliased where the layer is
        placed (affine or in perspective), on the CPU and the GPU alike; `.slop` 0.30
  - [x] Shape tools (U): Rectangle, Ellipse, Polygon (and stars), Line; fill and stroke in the
        options bar
  - [x] A vector layer's fill, stroke, corner radius, sides and star in the Properties panel
  - [ ] Pen and paths, vector masks
  - [ ] Type
- [ ] Patterns (ADR 0042)
  - [x] Pattern fill layers: an image source repeated across the plane, scaled and turned,
        sampled with wrapping on the CPU and the GPU alike; `.slop` 0.31
  - [x] The library (a folder of PNG files with an index, four generated patterns), Edit >
        Define Pattern, Layer > New Fill Layer > Pattern, the pattern picker, Properties
  - [x] Pattern Overlay (layer style)
  - [ ] Edit > Fill > Pattern, Pattern Stamp, the Patterns panel
- [ ] History memory limits
- [ ] Very large images (Phase 4)
- [ ] Distribution (Phase 5): installers built by CI on a version tag (done 2026-10-06, not
      code-signed); code signing, auto-update, user documentation
- [ ] macOS and Linux tested by hand (CI builds and tests them; nobody has used the app there)

## Phase 4 — Very large images

- [ ] Out-of-core tile store (disk cache), RAM/VRAM budgets
- [ ] Background computation with progress and cancellation
- [ ] Benchmarks on 100–500 MP documents, tracked in CI

## AI track *(research runs in parallel from Phase 1)*

- [ ] Benchmark set and experiment harness
      ([research notes](research/hd-generative-ai.md))
- [ ] Compare HD strategies (baseline, ROI, coarse-to-fine, tiled diffusion, HF re-injection)
- [x] 🔶 Inference runtime inside the app: ONNX Runtime in the helper, the generative graphs
      written by SlopShop (ADR 0025, ADR 0045)
- [ ] 🔶 Chosen HD pipeline, recorded with evidence
- [ ] AI node model: prompt, model + version, seed, mask, ROI, dependencies, cached result,
      stale/invalidated state, preview vs. full-definition render
- [x] First AI feature: object removal / generative fill in a region, optional and local
      (Delete's Generative Fill, ADR 0045)
- [ ] AI upscaling for enlargements (Photoshop's Preserve Details 2.0, Super Resolution), as a
      node: the classical detail pass was dropped for too small a gain (Phase 2)

## Open to contributors: advanced transforms

None of these exist yet, and none has a menu entry until it works (Edit > Transform lists only
what is implemented). Each must stay non-destructive and editable: parameters kept on the layer,
pixels never resampled into it (as [ADR 0017](adr/0017-non-destructive-transforms.md) does for
affine transforms), rendered identically on the CPU and the GPU. Discuss the model in an ADR
first.

- [ ] **Warp / mesh deformation** (Edit > Transform > Warp once it works): a grid of control
      points and handles over the layer, editable again at any time.
- [ ] **Puppet Warp**: a mesh over the layer's content with pins, fixed or moved, for organic
      deformations; pins and mesh kept, editable again.
- [ ] **Perspective Warp**: quadrilateral planes drawn on the image, connected, then moved
      together; planes and their positions kept, editable again. Distinct from the simple
      perspective of Free Transform below.
- [x] **Free Transform: distort and perspective** (corner handles moved freely, Ctrl and
      Alt+Shift+Ctrl as in Photoshop): a projective transform per pixel layer
      ([ADR 0038](adr/0038-projective-transforms.md)).
- [ ] **Groups in perspective**: Distort and Perspective on a group, each layer inside through
      the group's map (pixel layers refused past the horizon line; solid fills stay whole;
      gradient fills stop at the horizon; adjustment layers' masks follow). Only pixel layers
      are put in perspective today (`validate_transform` in `edit.rs`).
- [ ] **Selected pixels of a layer in perspective**: moving, copying and cutting them (the Move
      tool on a selection, Copy, Cut), and Smudge on such a layer. Refused with a message today
      (`affine_placement` in `app/src-tauri/src/paint.rs`): `PixelMove` and Smudge's field
      assume a constant scale in the layer's pixels.
- [ ] Proposed: **content-aware scaling** as an option of Free Transform's scaling (protecting
      what matters while the rest stretches), not a command of its own.
- [x] **Liquify**: brush deformations kept as a displacement field, an entry of the layer's
      stack, editable and removable ([ADR 0037](adr/0037-liquify.md)).

## Phase 5 — Extensibility and distribution

- [ ] 🔶 Plugin/extension API (nodes, importers/exporters)
- [ ] Scripting and batch processing through the CLI
- [x] Installers for Windows, macOS and Linux, built on a version tag (`release.yml`)
- [ ] Code signing, auto-update
- [ ] More interface languages (one catalog each)
