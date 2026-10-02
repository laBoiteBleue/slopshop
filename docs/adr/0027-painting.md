# 0027 — Painting: brush strokes on tiles

Status: proposed (2026-10-02; maintainer's choices: strokes computed on the CPU and shown by the
GPU, new paint layers in 8-bit sRGB, pen pressure from the first version).

## Context

Phase 3 ends with painting: the Brush (B) and the Eraser (E), then painting in layer masks and in
Quick Mask. The roadmap fixes Photoshop's model: a stroke rewrites the pixels of the layer it
paints on, and undo keeps the old tiles by reference (ADR 0003, 0005); non-destructive work paints
on an empty layer or in a mask. The brush must respect the selection from the start (ADR 0024).

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
4. **Every frame of a stroke is a real image**: the tiles whose coverage changed since the last
   frame are recomputed into a new `RasterImage` that shares every other tile (and the pyramid
   tiles above the unchanged ones) with the layer's image. The renderer displays that image in
   place of the layer's (a preview, not a document change). When the stroke ends, the same
   image becomes the layer's through **`Edit::SetLayerImage`**, whose inverse keeps the previous
   image: undo keeps exactly the old tiles that changed, by reference. Preview and result are
   the same pixels by construction.
5. **Tiles have an identity** for caches: the GPU tile cache and the `.slop` writer's tile
   hashes are keyed by the tile's allocation (the cache holds a reference, so the address
   cannot be reused while cached) rather than by the image and position. A stroke then uploads
   and hashes only the tiles it changed, and a shared uniform tile (empty layers, selections) is
   uploaded once. Composited display tiles (ADR 0022) are recomposited where the painted layer
   reaches, as after any edit of it; narrowing that to the changed tiles is a later
   optimization.
6. **Where it paints**: the active layer's pixels when it is a raster layer; its mask when the
   mask is the target (gray, black hides, white shows; with mask editing); the selection in
   Quick Mask. A transformed layer (ADR 0017) is painted in its own pixel grid: each pixel's
   center is mapped to the document through the layer's whole transform (groups included), so
   a dab stays round on the canvas whatever the scale or rotation. A stroke paints within the
   layer's bounds; growing a layer to follow the brush is future work. Fill and adjustment
   layers are not painted (Photoshop asks to rasterize; here, a notice).
7. **The selection limits the paint** where it is soft: its coverage at each pixel's document
   position multiplies the paint. No selection: everywhere.
8. **New empty layers** (Layer > New > Layer, Shift+Ctrl+N) are canvas-sized **8-bit sRGB
   with alpha**, one shared transparent tile until painted (Photoshop's default depth; the
   maintainer's choice). A layer keeps its own format when painted: a 16-bit or float layer is
   painted at its depth. The paint color is converted to the layer's space, explicitly; values
   outside an 8- or 16-bit layer's range are clipped by that format, as the user chose it.
9. **Colors**: a foreground and a background color (D: defaults, X: swap) with a color picker,
   chosen in sRGB for now and kept as working-space colors; the Eraser removes alpha.
10. **Input**: pointer events with pressure (Windows Ink pen, mouse at full pressure), their
    coalesced samples sent to the engine in batches at the display's rate (small JSON, never
    pixels); the stroke runs on a worker thread, never on the UI thread; the brush outline is
    drawn by the UI at the pointer.

## Alternatives

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

- Stroke cost is CPU time in proportion to the area of the dabs (measured and tracked in a
  benchmark); a frame recomputes and uploads only the tiles that changed.
- History memory grows with the tiles changed by strokes; limits and a history panel are
  roadmap items.
- `.slop` needs no change: a painted layer is a raster whose tiles are stored like any other.
- Order of work: the core (brush model, coverage, apply, `SetLayerImage`, tests and benchmark);
  tile identity in the renderer and the writer; the app (new layer, Brush and Eraser with their
  options bar, colors and picker, pressure, brush outline); then masks and Quick Mask.
