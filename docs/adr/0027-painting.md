# 0027 — Painting: brush strokes on tiles

Status: accepted, points 3 to 5 revised by [ADR 0029](0029-layer-stack.md) (2026-10-02; maintainer's choices: strokes computed on the CPU and shown by the
GPU, new paint layers in 8-bit sRGB, pen pressure from the first version, paint kept apart from
the layer's original pixels and mask and removable as a whole, the Eraser acting on alpha).

## Context

Phase 3 ends with painting: the Brush (B) and the Eraser (E), then painting in layer masks and in
Quick Mask. Photoshop's brush rewrites the pixels of the layer it paints on; non-destructive work
there means painting on an empty layer or in a mask. SlopShop's rule is non-destructive by
default (CLAUDE.md): an imported image must stay intact even when painted on, so the maintainer
chose to keep a layer's paint apart from its original pixels, removable as a whole (nothing else:
the paint has no transform, opacity or mode of its own). Undo keeps old tiles by reference (ADR
0003, 0005). The brush must respect the selection from the start (ADR 0024).

Constraints: performance first (a stroke follows the pen at the display's rate, whatever the
document size); never assume a layer fits in memory or VRAM; layers keep their own pixel format
and color space (ADR 0005, 0007); every change is an undoable edit; the engine works without the
UI (the same stroke gives the same pixels in tests, in the CLI and in the app, on any machine).

## Decision

1. **A stroke is computed on the CPU, tile by tile, and shown by the GPU.** The engine
   (`slopshop-core`) turns pointer samples into dabs and dabs into pixels on every core; the
   renderer only displays the result, through its usual path. One implementation, deterministic,
   identical headless and on any GPU. Rasterizing dabs on the GPU stays possible later, as an
   optimization of very large brushes, behind the same model.
2. **The brush model** (Photoshop's): a round tip of a *diameter* in document pixels and a
   *hardness* (full strength within `hardness × radius`, a smooth falloff to the radius, the
   edge anti-aliased by area); dabs every *spacing* × diameter along the path (25 % by
   default), the pointer samples interpolated in position and pressure; *flow* is how much each
   dab adds, *opacity* the most the whole stroke reaches. The pen's pressure scales the diameter
   and/or the opacity (two toggles, as in Photoshop's options bar). Brush shapes, angle,
   roundness, dynamics and smoothing come later as parameters of the same model.
3. **A stroke accumulates a coverage, not colors**: a per-stroke `f32` coverage in the painted
   layer's pixel grid, over the tiles the stroke touches only (`c ← c + flow·dab·(1 − c)`). The
   painted pixel is the layer's original pixel with the paint applied at `opacity × c ×
   selection`: source-over of the color (Brush) or alpha reduced (Eraser), in the document's
   blend space (ADR 0012), so that soft edges look as in Photoshop in perceptual documents.
   Recomputing from the original pixel and the coverage means dabs never re-quantize what
   earlier dabs wrote, and the stroke's opacity is a true cap.
4. **A raster layer keeps its original image and, once painted, its painted image**:
   `LayerContent::Raster { image, original: Option<Arc<RasterImage>> }`, where `image` is what the
   layer shows (the painted image once painted) and `original` the pixels it had before any
   paint. The painted image is the original with every stroke applied, in the same format, size and grid; it shares every tile
   no stroke touched (and the pyramid tiles above them), so it costs only the painted tiles. The
   layer shows its painted image when it has one: compositing, export and thumbnails read it in
   place of the original, and the layer's transform, mask, opacity and mode apply to it
   unchanged. A painted layer has a **mark in the layers panel** (a brush icon next to its
   name, a new empty layer included once painted). **Layer > Delete Paint** (also in the
   layers' right-click menu) removes it: the original comes back exactly. The original is never
   written. **A mask is painted the same way**: `LayerMask` keeps its image and, once painted,
   its painted image, shown in its place; the mark covers both, and Delete Paint removes the
   paint of the pixels and of the mask together.
5. **Every frame of a stroke is a real image**: the tiles whose coverage changed since the last
   frame are recomputed into a new image that shares every other tile with the layer's current
   pixels (painted, or original). The renderer displays it in place of the layer's (a preview,
   not a document change). When the stroke ends, the same image becomes the layer's painted
   image through **`Edit::SetLayerPaint`** (`None` deletes the paint), whose inverse keeps the
   previous one: undo keeps exactly the old tiles that changed, by reference. Preview and result
   are the same pixels by construction. The Eraser lowers the alpha of the painted image; Delete
   Paint brings back what it erased.
6. **Tiles have an identity** for caches: the GPU tile cache and the `.slop` writer's tile
   hashes are keyed by the tile's allocation (the cache holds a reference, so the address
   cannot be reused while cached) rather than by the image and position. A stroke then uploads
   and hashes only the tiles it changed, and a shared uniform tile (empty layers, selections) is
   uploaded once. Composited display tiles (ADR 0022) are keyed by the identities of the
   raster tiles they read, so a stroke frame recomposites only the display tiles over the
   tiles it changed (2026-10-04; before, every tile the painted layer reaches).
7. **Where it paints**: the active raster layer's painted image; its mask's painted image when
   the mask is the target (gray: white shows, black hides; the Brush paints the gray of its
   color, the Eraser and Delete the background color's, as Photoshop's: amended 2026-10-04 at
   the maintainer's request); Quick Mask's image likewise. A transformed layer (ADR 0017) is painted in its own pixel grid: each pixel's
   center is mapped to the document through the layer's whole transform (groups included), so
   a dab stays round on the canvas whatever the scale or rotation. A stroke paints within the
   layer's bounds; growing a layer to follow the brush is future work. Fill and adjustment
   layers are not painted (Photoshop asks to rasterize; here, a notice).
8. **The selection limits the paint** where it is soft: its coverage at each pixel's document
   position multiplies the paint. No selection: everywhere.
9. **New empty layers** (Layer > New > Layer, Shift+Ctrl+N) are raster layers whose original is
   a canvas-sized **8-bit sRGB** transparent image, one shared tile (Photoshop's default depth;
   the maintainer's choice); their paint is deleted like any other. A layer is painted in its
   own format: a 16-bit or float image at its depth, a gray image in gray (the color becomes its
   luminance, as in Photoshop's Grayscale mode). The paint color is converted to the layer's
   space, explicitly; values outside an 8- or 16-bit layer's range are clipped by that format.
10. **Colors**: a foreground and a background color (D: defaults, X: swap) with a color picker,
    chosen in sRGB for now and kept as working-space colors. The Eraser acts on alpha: the
   layer's, or the mask's coverage.
11. **Input**: pointer events with pressure (Windows Ink pen, mouse at full pressure), their
    coalesced samples sent to the engine in batches at the display's rate (small JSON, never
    pixels); the stroke runs on a worker thread, never on the UI thread; the brush outline is
    drawn by the UI at the pointer.

## Alternatives

- **Photoshop's model, the paint written into the layer's pixels**: the most familiar, but an
  imported image is lost once the document is saved; non-destructive work then depends on the
  user remembering to add a layer.
- **A layer created above automatically at the first stroke**: protects the image, but the
  Eraser no longer erases the image and the panel fills with layers nobody asked for.
- **The paint as a separate delta (a color layer and an erase mask) composited by the
  renderer**: the paint would survive a change of the original, but a second and third raster
  per layer in every compositing path; the painted image costs the same memory and nothing in
  the renderer. It can come with the DAG (ADR 0003) if originals ever change under paint.

- **Strokes on the GPU**, the CPU as a reference for tests, the touched tiles read back at the
  end: faster for very large brushes, but two implementations to keep identical, a readback per
  stroke, and stored pixels that may differ slightly from one GPU to another.
- **A wet stroke layer composited by the shader** (the coverage uploaded as a mask, the layer
  untouched until the end): less CPU work per frame, but a second compositing path to keep
  exact, and the preview is not the result.
- **Accumulating dabs straight into the layer's pixels**: simplest, but an 8-bit layer
  re-quantizes at every dab (banding in soft strokes) and the stroke opacity is no cap.
- **New layers in 16-bit or float**: no banding in very soft gradients, twice the memory of
  painted areas; a per-document default can come later.

## Consequences

- *Gradient tool (2026-10-05)*: a paint may lay a color that varies across the canvas
  (`Paint::Gradient`, `PaintOp::Gradient`): each pixel takes the gradient's color at its place
  in the document and its opacity scales the amount, on a layer's stack as on a coverage (its
  grays). It is laid as Edit > Fill is, the same delta `P + k·B`: nothing new is stored.
- Stroke cost is CPU time in proportion to the area of the dabs (measured and tracked in a
  benchmark); a frame recomputes and uploads only the tiles that changed.
- History memory grows with the tiles changed by strokes; limits and a history panel are
  roadmap items.
- `.slop` gains a compatible addition: the optional painted images of a raster and of a mask (a new node
  version, so that an older SlopShop reports a newer file instead of dropping the paint). Shared
  tiles are stored once (the file addresses tiles by content). PSD export writes the painted
  pixels; PSD import has no paint.
- The Move tool dragged inside a selection moves the selected pixels the same way
  (`slopshop_core::move_pixels`): the result is the layer's (or its mask's) painted image,
  computed from the pixels the move started from, so the original stays intact. The layer grows
  (by large steps of whole tiles, its original, mask and transform with it) to keep the pixels
  moved beyond it, off the canvas too.
- Order of work: the core (brush model, coverage, apply, the painted image and `SetLayerPaint`,
  tests and benchmark); tile identity in the renderer and the writer, `.slop`; the app (new
  layer, Brush and Eraser with their options bar, colors and picker, pressure, brush outline,
  Delete Paint and a mark on painted layers); then masks and Quick Mask.
