# 0044 — Warps: a deformation mesh placing a layer

Status: proposed (2026-10-07, drafted for the maintainer, who approved the warps the same day;
the roadmap asks for an ADR first; the questions at the end are theirs).

## Context

The roadmap's "advanced transforms" left three of Photoshop's deformations to do, each to stay
non-destructive and editable, rendered identically on the CPU and the GPU:

- **Warp** (Edit > Transform > Warp): a grid of control points and handles (Bézier patches,
  3 × 3 by default, finer grids on request), and presets (Arc, Bulge, Flag, Wave…).
- **Puppet Warp**: a mesh over the layer's visible pixels, pins placed and moved, the rest
  following as rigidly as possible (as-rigid-as-possible deformation).
- **Perspective Warp**: quadrilaterals drawn on the image (Layout mode), joined at their edges,
  then their corners moved (Warp mode); each quad a homography.

Today a layer is placed by one projective map (ADR 0017, ADR 0038), and Liquify keeps a
displacement field in the layer's stack (ADR 0037), within the layer's pixels.

All three deformations end as the same thing: **a mesh of small pieces, each mapped from the
layer's content to its parent smoothly**. The GPU draws a textured mesh natively; the CPU can
draw the same triangles exactly.

## Decision (proposed)

1. **A layer's placement gains an optional warp**: `Layer::warp`, applied in the content's
   space before the layer's transform (so Free Transform still moves, scales and puts a warped
   layer in perspective). Its parameters are kept, never the warped pixels:
   - `Grid`: control points and handles of a Bézier patch grid (Warp, and its presets as
     starting grids);
   - `Puppet`: the mesh's density and expansion, the pins and where they were moved
     (the deformation computed from them, as-rigid-as-possible, deterministic);
   - `Planes`: the quads in layout and where their corners went (Perspective Warp).
2. **Drawn as triangles**: each warp gives a triangle mesh from the content's coordinates to
   the deformed ones, fine enough that each triangle is close to its exact map (subdivided where
   the deformation bends). The renderer draws it, each triangle sampling the layer through its
   own affine map with the EWA resampling of ADR 0018 (its Jacobian per triangle); the CPU
   compositor rasterizes the same triangles with the same coverage rule. Tests compare them.
3. **Masks follow** (linked, as for transforms); layer styles draw from the warped shape.
4. **Painting on a warped layer** goes through the mesh backward (each brush dab mapped into the
   layer's pixels, as through a projective map, ADR 0038); where the mesh folds over itself, the
   topmost piece is the one painted. Moving selected pixels of a warped layer is refused at first,
   as for layers in perspective.
5. **Editing**: Edit > Transform > Warp shows the grid over the layer in Free Transform's
   overlay (handles, a preset menu and the grid size in the options bar); Edit > Puppet Warp and
   Edit > Perspective Warp are modes of their own, like Liquify's workspace, with Enter and Esc;
   each session one undo entry. Rasterize bakes the warp like a transform.
6. **`.slop`**: `params.warp` (kind and parameters) on the nodes that have one, a new node version
   so that older readers refuse them. PSD: Warp's grid is read from and written to the smart
   object transform of a placed layer when there is one; otherwise the pixels are exported warped.
7. **Order**: Warp (a grid, the presets), then Perspective Warp (planes are homographies,
   close to ADR 0038), then Puppet Warp (the as-rigid-as-possible solver).

## Alternatives

- **A displacement field in the stack** (as Liquify): already editable, but it moves pixels
  within the layer's grid only (a warp can push content far outside it), and a field can't be
  edited again as a grid of handles.
- **Resampling into the layer when the warp is applied** (Photoshop on pixel layers): simple,
  but destructive.
- **An inverse map evaluated per pixel** (inverting the Bézier patches): exact, but expensive and
  ill-defined where the warp folds; triangles are what the GPU does best.

## Consequences

- The renderer and the CPU compositor gain a mesh path beside the projective one; bounds, hit
  testing and the display cache use the mesh's.
- Free Transform's overlay gains the grid; two new workspaces (Puppet, Perspective).
- The deformation of groups (a warp on a group) is left open, as perspective on groups is.

## Questions for the maintainer

1. Warp on every kind of layer from the start (pixel, fill, group, later vector), or pixel
   layers first?
2. Photoshop's Warp presets (15 of them): all at once, or a few (Arc, Bulge, Flag, Wave, Twist)?
3. Puppet Warp's mesh: from the layer's visible pixels (Photoshop's), or a plain grid over its
   box (simpler, less natural)?
4. Painting on a warped layer: allowed (through the mesh, as proposed), or refused at first?
5. The order: Warp, Perspective Warp, Puppet Warp?
