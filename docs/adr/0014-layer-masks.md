# 0014 — Layer masks

Status: accepted (2026-09-30).

## Context

Masks hide parts of a layer without touching its pixels: the base of non-destructive
compositing, of future selections (a selection becomes a mask) and of adjustment layers. There
is no brush yet, so the first way to make a mask is the one the maintainer chose: from the
layer's transparency (Photoshop's Layer > Layer Mask > From Transparency).

## Decision

1. **A layer has at most one mask**: a gray raster at the document origin (like raster layers),
   with `enabled` and `replaces_alpha` flags. Its samples are coverage: 0 hides, 1 shows, read
   linearly (the mask image has a linear transfer, so 50% is 50% coverage); values are clamped
   to `[0, 1]`. Outside the mask image the layer is hidden. It is immutable and shared like any
   raster; the layer's own pixels never change.
2. **Coverage**: the layer's premultiplied color is multiplied by the mask value before
   blending (ADR 0012), after opacity. A disabled mask is kept and ignored.
3. **From transparency**: the mask is the layer's alpha channel, copied tile by tile in its own
   sample type (8/16-bit or float), and `replaces_alpha` is set: while the mask exists, enabled
   or not, the layer's own alpha is ignored, as Photoshop moves transparency into the mask.
   Disabling the mask therefore shows the whole layer; deleting the mask gives the layer its
   own alpha back. Where a source stores premultiplied colors, fully transparent pixels have no
   color left and show black.
4. **Edits** `SetLayerMask` (add, replace, delete) and `SetLayerMaskEnabled`, undoable.
5. **Both compositors** (CPU reference, GPU) apply masks with the same math, checked by tests.
   On the GPU a mask is planned, cached and area-sampled like a raster layer.
6. **`.slop` schema 0.3**: node parameters at version 3 carry `mask` (image key, `enabled`,
   `replaces_alpha`), so that older readers refuse files they would render wrongly; mask images
   are stored like layer images (content-addressed tiles, shared when identical).

## Alternatives

- **Rewriting the layer's alpha** (Photoshop's way): destructive, and impossible for our
  immutable, shared rasters.
- **Masks as separate layers clipping the layer below**: more general, but that is what groups
  and clipping will do; a mask belongs to its layer.

## Consequences

- Layer views carry the mask state; the layers panel shows a mask thumbnail (Shift+click
  toggles it, as in Photoshop) and the Layer menu gets Layer Mask commands.
- Painting in masks comes with brushes (Phase 3); selections will create masks.
- A document with a mask on its bottom layer is no longer structurally opaque for export
  defaults: its alpha is kept.
