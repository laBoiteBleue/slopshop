# 0029 — A layer's own stack: paint and applied effects

Status: accepted (2026-10-03, validated by the maintainer; their choices: what Photoshop does destructively goes on a
stack inside the layer, whose entries are not editable but can be deleted, in their order; files
and history keep only paint deltas and parameters, never a re-stored copy of every tile).
Revises [ADR 0027](0027-painting.md), points 3 to 5.

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
   tile; a fill without a selection is one uniform tile. `P` keeps the layer's pixel format (an
   8-bit layer's paint quantizes as today), `k` is 16-bit. Painting continues the top entry when
   it is paint, otherwise starts a new one.
3. **Tools that read pixels bake what they read.** Moving selected pixels (and later Smudge,
   Clone, AI fill) write into `P` the values they took from below: deleting an effect under
   them does not change those values.
4. **Effects are parameters.** Image > Adjustments adds an effect entry to each selected
   visible raster layer (one undo entry); its parameters are those of the adjustment layers
   (ADR 0020), the math is the same, applied to the layer's own color, before its mask,
   opacity and blend mode. The selection limits it: the entry keeps the selection by reference
   (no copy) with the layer's transform at that moment, so the effect stays where it was
   applied when the layer moves. Groups, hidden, fill and adjustment layers are not eligible, as
   in Photoshop. Filters arrive later as effects of the same kind.
5. **Entries are not edited and not reordered.** Deleting one recomputes what is above it;
   neighbours that become alike merge: two paints exactly (point 2), two effects of the same
   kind as one entry holding both in order (the same pixels, one mark), two Inverts cancel.
   The Restore Eraser (a new tool) brings back the original through every paint entry where it
   rubs (`P ← (1−r)·P`, `k ← (1−r)·k + r`); effects stay applied.
6. **Evaluation** is a cache, never saved: the layer's result per tile and pyramid level, in a
   bounded tile cache like the display cache (ADR 0022), computed on the GPU from the original
   and the stack (the CPU reference compositor runs the same math for export and tests). Level
   0 is exact; coarser levels evaluate the stack on the original's coarser level, the
   approximation views of adjustment layers already make. A spatial filter's result is cached
   too, so that deleting an entry above it recomputes from there. Layers without a stack skip
   all of this.
7. **Edits**: `Edit::PushEntry`, `Edit::RemoveEntry` (its merges included) and
   `Edit::SetTopPaint` (a stroke's end, replacing the top paint), each inverse keeping the
   previous entries by reference. Layer > Delete Paint becomes deleting entries (the UI is the
   maintainer's call: a small mark on the layer shows that it has entries).
8. **`.slop`**: raster nodes gain `params.stack` (paint: the keys of `P` and `k` and the blend
   space; effect: kind, parameters, selection key and transform), a new node version. A v6
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
