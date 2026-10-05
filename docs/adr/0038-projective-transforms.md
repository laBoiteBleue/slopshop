# 0038 — Projective layer transforms: Distort and Perspective

Status: accepted (2026-10-05; the maintainer chose the direction, option A, after the toolbar
audit, and started the work once the display performance work and the toolbar were done).
Amends [ADR 0017](0017-non-destructive-transforms.md) (point 1) and
[ADR 0018](0018-resampling.md) (point 4).

## Context

Photoshop's Free Transform also distorts (each corner dragged freely) and puts in perspective
(two corners moving symmetrically). The toolbar audit decided these go into Edit > Transform
and Free Transform's box, non-destructively and editable afterwards, rather than as a mode of
the Crop tool. A corner dragged freely makes a map that is not affine: straight lines stay
straight, parallel ones do not. That is a projective map (a homography, a 3 × 3 matrix up to
scale).

Today `Layer::transform` is an affine map of six numbers (ADR 0017), and much relies on it
being affine (a survey of the code, 2026-10-05):

- **Resampling** (ADR 0018, point 4): the pyramid level, the texel map and the EWA ellipse are
  computed once per layer because the Jacobian is constant; the GPU receives a 2 × 3 matrix
  and a constant ellipse (`composite.wgsl` `resample`).
- **Bounds**: about fourteen `map_rect` calls map a box through the transform or its inverse;
  forward ones stay exact for a homography (a convex quad maps to a convex quad), inverse ones
  of a document area are unbounded where the area crosses the horizon line.
- **Constant linear part**: moving selected pixels, layer styles' coverage cache, the paint's
  anti-aliasing width (the determinant), Free Transform's decomposition into rotation, skew and
  scale (`transformValues.ts`).
- **Formats**: `.slop` stores six numbers (node version 5) for a layer's transform and for a
  stack step's `to_document`; the IPC and the UI's `Matrix` are six numbers.

Whole-pixel translations (most layers) have their own exact fast paths everywhere; they stay.

## Decision

1. **One transform per layer, projective.** `Layer::transform` becomes a homography
   (`Projective`, nine numbers, normalized so that the last is 1), the affine maps a special
   case (`is_affine`: its last row `0 0 1`). Groups compose as today. Masks still follow their
   layer. A layer whose transform puts any of its pixels at or beyond the horizon (`w ≤ 0`
   over its bounds) is refused by the edit, as a non-invertible affine map is today.
2. **Fast paths kept, in this order**: whole-pixel translation (texel shift), affine (the
   current per-layer constants, unchanged), projective (per pixel). Nothing changes for
   documents without a projective layer, in speed or in bits.
3. **Resampling a projective layer**: the inverse maps each output pixel center to the source
   (with the divide); the EWA ellipse comes from the Jacobian at that pixel (ImageMagick's
   perspective distort, the reference ADR 0018 already follows), bounded so that a pixel near
   the horizon line never reads more than a fixed number of texels (`MAX_EXTENT`). One pyramid
   level is read for the whole layer, the finest any of its corners needs (as built: the GPU
   plans the tiles of one level per layer; where the layer recedes most, the capped ellipse
   reads that level). The CPU stays the reference; the GPU computes the same (the uniform gains
   the third row and a flag), tested pixel for pixel against it as ADR 0018 does.
4. **Bounds**: forward mapping of a layer's box stays `map_rect`'s four corners (exact for a
   convex quad); inverse mapping of a document area is clipped to where the layer lies before
   it is mapped, so it stays bounded.
5. **What reads a layer through its transform** (painting in the layer's grid, the Clone
   Stamp and healing, moving pixels, picking, snapping, layer styles, stack steps'
   `to_document`, Rasterize, export) works per pixel already or gets the projective map; the
   few that need a constant linear part (moved pixels' offset, the styles' cache, the paint's
   anti-aliasing width) use the local Jacobian or fall back to their general path.
6. **Free Transform**: a corner dragged with Ctrl distorts (only that corner moves);
   Alt+Shift+Ctrl perspective (the opposite corner on that side moves symmetrically), as in
   Photoshop; Edit > Transform > Distort and Perspective open the box in those modes. Its
   fields (X, Y, W, H, angle, skew) show while the map is affine and are grayed once it is not.
   The outline stays a quad of the four corners; snapping applies to the corners.
7. **Formats**: `.slop` writes nine numbers only for a projective transform, at a new node
   version (older readers refuse such a node rather than misplace it, as for version 5); affine
   ones are written as today. Stack steps' `to_document` likewise. The IPC carries nine numbers.
   PSD export renders the layer through it, as for any transform; no PSD import change.

## Alternatives

- **An optional corner-pin stage after the affine map** (`Layer::distort`): fewer types
  changed at first, but two notions of transform everywhere, and Free Transform composing
  them; more complexity for the same result.
- **Distortion as an entry of the layer's stack** (a field, as Liquify): little code, but
  [ADR 0034](0034-editable-operations.md) declined the transform as a stack entry, and the
  pixels would be resampled into the layer.
- **A Perspective Crop tool**: the audit declined it; the same result comes from Perspective
  then Crop, editable.

## Consequences

- The work comes in steps, each with its tests: the type and its fast paths (no behavior
  change); CPU resampling of projective layers; GPU resampling, tested against the CPU; `.slop`
  and the IPC; Free Transform's gestures and Edit > Transform's entries.
- The GPU compositor changes (`composite.wgsl`); scheduled after the display performance work
  that changes it too (maintainer, 2026-10-05).
- Perspective Warp (several quads) and Warp (a mesh) stay separate items for contributors
  (roadmap): they are not projective.
