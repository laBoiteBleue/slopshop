# 0029 — A layer's own stack: paint and applied effects

Status: accepted (2026-10-03, validated by the maintainer; their choices: what Photoshop does destructively goes on a
stack inside the layer, whose entries are not editable but can be deleted, in their order; files
and history keep only paint deltas and parameters, never a re-stored copy of every tile).
Revises [ADR 0027](0027-painting.md), points 3 to 5. Points 3 to 5 revised by
[ADR 0034](0034-editable-operations.md): entries are editable, tools that read pixels replay.

## Context

Layers work as in Photoshop: an adjustment layer changes what is below it, within its group.
Other operations rewrite a layer's pixels in Photoshop: the Brush, the Eraser, Edit > Fill and
Stroke, Delete and Cut with a selection, moving selected pixels, Image > Adjustments, and the
filters to come (blur, AI). ADR 0027 keeps a raster layer's original and one painted image
holding all of its paint: the original is safe, but every operation that is not paint (an
applied adjustment, a filter) would have to be baked into that image, a full copy of every tile
it reaches, and could never be removed once painted over.

The maintainer wants those operations kept as a stack inside the layer, without the cost and
the complications of re-editing them (a blur under paint): an entry is removed, never changed.

## Decision

1. **A raster layer holds its original and a stack of entries**, bottom to top, in the order
   they were applied: `original → entry → entry → …`. Two kinds of entries:
   - **Paint**: what painting tools and pixel commands did (point 2).
   - **Effect**: an applied adjustment or filter, kept as its parameters (point 4).
   Fill and adjustment layers, groups and masks have no stack (a mask keeps its painted image,
   ADR 0027).
2. **Paint is a delta, not pixels.** A paint entry turns the result `B` of what is below it into
   `P + k·B`, per pixel: `P` a premultiplied color, `k` a factor in `[0, 1]`, in the blend space
   recorded with the entry (ADR 0012). The Brush is `P ← a·C + (1−a)·P`, `k ← (1−a)·k`; the
   Eraser (alpha paint) multiplies both by `1−a`. Any sequence of brush and eraser work stays of
   this form, so paint over an effect follows when an effect below it is deleted, and two paint
   entries that become neighbours merge exactly (`P₂ + k₂·P₁`, `k₂·k₁`). A paint entry stores
   only the tiles it touched; untouched tiles are the identity (`P = 0`, `k = 1`), one shared
   tile; a fill without a selection is one uniform tile. `P` is stored as a pixel of the
   layer's own format with alpha (an 8-bit layer's paint quantizes as today, and a v6 painted
   pixel is kept bit for bit), converted to the blend space to compute; `k` has `P`'s sample
   type (`f32` for float layers), so that `P`'s alpha and `k` round to complementary values and
   an opaque pixel painted over stays exactly opaque. The result is quantized to the layer's
   format after every entry, as Photoshop writes the layer's pixels after every operation: then
   evaluating from the original and adding to a result give the same pixels, and only the tiles
   an entry reaches are recomputed. Painting continues the top entry when it is paint, otherwise
   starts a new one.
3. **Tools that read pixels bake what they read.** Moving selected pixels (and later Smudge,
   Clone, AI fill) write into `P` the values they took from below: deleting an effect under
   them does not change those values.
4. **Effects are parameters.** Image > Adjustments adds an effect entry to each visible raster
   layer (one undo entry; every one shown, not only the selected ones: the maintainer's
   choice, 2026-10-03); its parameters are those of the adjustment layers
   (ADR 0020), the math is the same, applied to the layer's own color, before its mask,
   opacity and blend mode. The selection limits it: the entry keeps the selection by reference
   (no copy) with the layer's transform at that moment, so the effect stays where it was
   applied when the layer moves. Groups, hidden, fill and adjustment layers are not eligible, as
   in Photoshop. Filters arrive later as effects of the same kind.
5. **Entries are not edited and not reordered.** Deleting one recomputes what is above it;
   neighbours that become alike merge: two paints exactly (point 2), two effects of the same
   kind as one entry holding both in order (the same pixels, one mark), two Inverts cancel.
   Contiguous entries are never alike (maintainer, 2026-10-03): painting continues the top
   paint, and an effect of the kind of the top entry joins it when applied.
   The Restore Eraser (a new tool) brings back the original through every paint entry where it
   rubs (`P ← (1−r)·P`, `k ← (1−r)·k + r`); effects stay applied.
6. **Evaluation** is a cache, never saved: the layer's result per tile and pyramid level, in a
   bounded tile cache like the display cache (ADR 0022), computed on the GPU from the original
   and the stack (the CPU reference compositor runs the same math for export and tests). Level
   0 is exact; coarser levels evaluate the stack on the original's coarser level, the
   approximation views of adjustment layers already make. A spatial filter's result is cached
   too, so that deleting an entry above it recomputes from there. Layers without a stack skip
   all of this. *First version (`slopshop_core::stack`, 2026-10-03)*: the result is evaluated on
   the CPU, on every core, into an image held by the layer as its painted image is today
   (tiles no entry reaches are the original's), so that the renderer, export, thumbnails and
   tools read it unchanged; history and files still keep only entries. The GPU evaluation and
   the bounded cache come as an optimization (measured: an adjustment over a 12 MP 8-bit layer,
   pyramid included, 0.28 s on 32 threads). A stroke evaluates what is below its paint once
   per tile, on every core, and only where the paint it continues lies (elsewhere it is what
   the layer shows); the paint keeps those tiles for the next stroke while the stack below it
   is the same. Measured on a 12 MP 8-bit layer, 32 threads, a 200-pixel brush: 3.3 ms a frame
   painting the pixels themselves (ADR 0027), 6.4 ms on a stack, 6.9 ms (12.7 ms at worst)
   continuing paint over three effects. *GPU (2026-10-03, maintainer's request)*: an edit of a
   stack evaluates nothing: the layer's pixels (`stack::Pixels`) are a cache filled when first
   asked, by what needs them (tools, thumbnails, the clipboard, export) or by a thread of their
   own the display starts; meanwhile the shader evaluates the stack itself (the original, then
   each paint `P + k·B` and each effect within its selection, before the layer's mask, opacity
   and mode), and the display cache keeps the composited tiles. At 100 % it shows the CPU's
   pixels but for rounding (the CPU rounds after each entry); zoomed out it evaluates the stack
   on the coarser level of each image, close but for the edges of paint under nonlinear
   effects; the frame asks to be shown again until the exact pixels are there, then shows
   them. Applying an effect, deleting an entry, undo and redo are instant whatever the image.
   Export evaluates the pixels first (exact).
7. **Edits**: one edit, `Edit::SetLayerStack` (implemented so, 2026-10-03), gives a layer a
   stack built by the stack's own operations (an effect added, an entry deleted with its
   merges, the top paint set by a stroke, the layer grown): its inverse keeps the previous
   stack by reference, never evaluated pixels, and undo evaluates again only the tiles the
   entries that differ reach. Layer > Delete Paint becomes deleting entries (the UI is the
   maintainer's call: a small mark on the layer shows that it has entries).
8. **`.slop`**: raster nodes gain `params.stack` (paint: the keys of `P` and `k` and the blend
   space; effect: kind, parameters, selection key and transform), a new node version (v7,
   schema 0.12, `params.image` being the original). A v6
   painted image converts exactly: one paint entry with `P` the painted pixels and `k = 0` where
   they differ from the original, the identity elsewhere. PSD export writes the evaluated
   layer; PSD import has no stack.

## Alternatives

- **One painted image (ADR 0027 as is)**: no renderer change, but applied adjustments and
  filters must be baked into it, a copy of every tile they reach, and cannot be removed once
  painted over.
- **Editable entries (Photoshop's smart filters, with paint between them)**: every change
  recomputes everything above it (a blur under paint on a large image), and the paint above
  was made looking at the old result.
- **Each operation as a new layer**: the layers panel fills with layers nobody asked for, and
  the Eraser no longer erases the image (rejected in ADR 0027).

## Consequences

- The renderer evaluates a layer's stack into cached tiles; drawing a stroke shows the top paint
  entry of each frame in place of the layer's, as painted images are shown today.
- A layer with many entries costs evaluation time when it changes, not per frame (cached).
- Paint blend modes (Multiply…) are not of the form `P + k·B`: when they come, a paint entry
  carries a mode, and only Normal neighbours merge.
- Edit > Fill, Stroke, Delete and Cut with a selection are paint; Image > Adjustments are
  effects; the editable stroke is a layer of its own (Layer menu), later.
- Order of work: the core model, edits and CPU evaluation with tests; `.slop` with the v6
  conversion; the renderer's cache; the app (Restore Eraser, entries mark, Image >
  Adjustments); then the Edit commands that paint.
