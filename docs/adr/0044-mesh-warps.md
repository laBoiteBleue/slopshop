# 0044 — Warps: a deformation mesh placing a layer

Status: accepted (2026-10-07, the maintainer's answers below).

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

## Decision

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

## Answers (the maintainer, 2026-10-07)

1. Pixel layers first; the other kinds later.
2. Five presets to start: Arc, Bulge, Flag, Wave, Twist.
3. Puppet Warp's mesh from the layer's visible pixels, as Photoshop.
4. Painting on a warped layer refused at first, as on layers in perspective (point 4 then
   applies later).
5. Order: Warp, Perspective Warp, Puppet Warp.

## Implementation of Warp, part 1 (2026-10-13)

Status: accepted (2026-10-13, under the maintainer's delegation; to review).

- **Model**: `Layer::warp: Option<Arc<Warp>>`, a grid of `columns × rows` bicubic Bézier patches
  (1 to 16 each way) over a `frame` of the content: the box of the layer's visible pixels when
  the warp is made (Photoshop warps what shows), kept afterwards. Content outside the frame is
  not drawn while the layer is warped. The grid remembers the style it was made from (`None`
  once edited point by point, Photoshop's Custom).
- **Styles**: each is a deformation of the frame (Arc: the center line bent into an arc of
  `bend × 180°`, its length kept, the other lines concentric; Bulge: the top and bottom edges
  pushed out in the middle; Flag: one period of a sine; Wave: a sine whose phase shifts from
  top to bottom; Twist: turned about the center, the more the closer to it), applied to the
  control points of a plain 3 × 3 grid; Vertical swaps the axes. The grid is an approximation
  of the deformation, then the only truth (it is what is drawn and saved).
- **Mesh**: each patch cut into 12 × 12 cells, two triangles each; the border extended by the
  resampling filter's reach (the border patches extrapolated, at most a quarter of a patch) so
  that the layer's edges are antialiased as an unwarped layer's are. Bins over the document
  find the triangle under a pixel; where the warp folds over itself, the last triangle listed
  wins (CPU and GPU alike).
- **Resampling**: one pyramid level for the whole layer, the finest any triangle needs; each
  sample's ellipse from its triangle's affine map (ADR 0018). The GPU reads the mesh from the
  tile table, as it reads adjustments' lookup tables: no new binding.
- **Display**: a warped layer whose stack is being evaluated shows what it showed before, warped,
  rather than the display's stack steps (not warped).
- **Refused at first**: painting, moving, copying or floating a warped layer's pixels (and its
  mask's), with a message; Rasterize bakes the warp into the pixels when the layer has no mask
  (with one, the mask would need warping too: later).
- **Editing**: Edit > Transform > Warp shows the grid in Free Transform's place (not combined
  with the box's handles yet) and the options bar's Warp (None and the five styles),
  orientation and Bend (-100 to 100 %, 50 % when a style is first chosen, as in Photoshop);
  each change is live, Enter applies (one undo entry), Esc or undo cancels. Dragging the grid's
  points and handles, and grid sizes, come next.
- **`.slop`**: node version 18 (schema 0.35); serde_json reads numbers back to the bit
  (`float_roundtrip`), so that a grid survives a save exactly.

## Implementation of Warp, part 2: editing the grid (2026-10-15)

Status: accepted (2026-10-15, under the maintainer's delegation; to review).

- **On the canvas**, as in Photoshop: the grid shows its anchors (the patches' corners) and
  their handles (the other control points on the patches' edges). Dragging an anchor moves it
  with its handles; dragging a handle moves it alone; dragging inside a patch (or on a line
  between anchors) bends it: the patch's corners stay and its handles move the least they can
  (each by its weight at the point grabbed, a least-norm solution), so that the point under
  the pointer follows it exactly. The grid then is Custom (no style).
- **The points inside a patch** are not shown: when the edges move, they follow as a Coons
  patch of the edges does (bilinearly blended), by the change only, so that a style's grid
  does not jump when first edited and an affine move of the edges moves them alike.
- **Grid size** (options bar: Default 3 × 3, 4 × 4, 5 × 5; any other grid shows Custom): a
  warped grid is refitted at the new size, each new patch interpolating the old surface at
  4 × 4 points (exact for whole subdivisions and plain grids); without a warp, a plain grid of
  that size. Choosing a style makes a 3 × 3 grid again, as Photoshop's styles ignore the grid.
- **Split Warp** Crosswise, Vertically, Horizontally (options bar buttons): the next click on
  the grid cuts the patches it crosses there (de Casteljau, the shape unchanged), one split per
  click, up to 16 patches a side. Removing a split, and moving several points at once, are not
  done yet.
- **The UI edits, the engine validates**: as Free Transform computes its matrix in the UI, the
  grid's edits are geometry on its control points (`app/src/lib/warpGrid.ts`); each step sends
  the whole grid (`warpLayer` with `grid`), replacing the gesture's previous step, so that
  Enter makes one undo entry. `warp_grid` answers the layer's control points, frame and
  placement (the overlay draws the grid from them; the engine's styles still make their
  grids). Edit > Transform > Warp on a warped layer shows its current grid, Custom or a style,
  editable again.
- **Vector and text layers**: not warped yet. They are drawn from their geometry at the
  document's resolution (ADR 0041, the text engine), not from pixels the mesh can resample;
  warping them means warping their outlines (Photoshop converts a shape's path; Warp Text is a
  text property of its own), a change to their drawing path rather than to the warp.

## Implementation of Warp, part 3: the other styles (2026-10-15)

Status: accepted (2026-10-15, under the maintainer's delegation; to review).

- Photoshop's other ten styles join the five, in its menu's order: Arc, Arc Lower, Arc Upper,
  Arch, Bulge, Shell Lower, Shell Upper, Flag, Wave, Fish, Rise, Fisheye, Inflate, Squeeze,
  Twist. As the first five, each is SlopShop's own approximation, a deformation of the frame
  applied to a plain 3 × 3 grid's control points (in `slopshop_core::warp`, documented there):
  Arc Lower and Upper bend one edge, the other straight; Arch raises every line alike; the
  Shells are the Arcs with the far edge drawn in; Fish waves the top and bottom edges in
  opposite ways; Rise lifts the lines from one side to the other; Fisheye pushes out from the
  center, the corners staying; Inflate pushes every edge out in its middle; Squeeze draws the
  sides in and stretches along the middle.
- `.slop`: the new identifiers are written in `preset.style` (schema 0.35 is not released yet);
  a style a reader does not know now reads as none (the grid edited point by point), the points
  being what is drawn, rather than refusing the file.

## Rasterize of a warped layer with a mask (2026-10-15)

Status: accepted (2026-10-15, under the maintainer's delegation; to review).

Rasterize now bakes a warped layer that has a mask: its mask is warped alike, into the new
content space of the baked pixels. The plan composites a second scratch document: a layer
showing the mask as its alpha (the mask as its pixels and its mask, its own alpha ignored),
warped by the same grid, both grown by a tile each way so that the layer is opaque past the
warp's edges and only the mask's own edges fade, as on screen. The composite's alpha is the new
mask (`RasterImage::alpha_mask`); the mask keeps whether it is enabled and replaces the layer's
alpha. Tested against the display: the same pixels before and after.

## Implementation of Perspective Warp, part 1 (2026-10-19)

Status: accepted (2026-10-19, under the maintainer's delegation; to review).

- **Model**: `Warp` is an enum: `Grid` (Warp's Bézier grid, unchanged) or `Planes` (Perspective
  Warp's). Planes are corners where they were laid out and where they were moved (content
  pixels), each plane four indices into them, over the same `frame` as a grid (the layer's
  visible pixels when first warped). Planes listing the same two corners are joined along that
  edge: moving a corner moves every plane it belongs to. A layer has one warp: Warp refuses a
  layer warped by planes and Perspective Warp a layer warped by a grid, each with a message
  saying what edits it or that Rasterize bakes it first (Photoshop would warp the warped
  pixels; a kept warp cannot hold both).
- **Map**: each plane maps its quad as laid out to its quad as moved by a homography. Two
  homographies that agree on an edge's two corners send the edge onto the same segment but
  spread its points differently, which would tear the image along a joined edge. Each plane's
  map is therefore its homography plus a correction along its joined edges (the mean of the
  joined planes' images of each edge point minus its own, on the same segment: the edge stays
  straight), blended inward as a Coons patch blends its edges, so that it vanishes at the
  corners and on the free edges; inside a plane, away from joined edges, the map is the
  homography. The content outside the planes follows them (in Photoshop the whole layer
  moves): each plane extended beyond its edges by its map's first order there (never through a
  horizon line, whatever the perspective), the planes weighted by the inverse fourth power of
  their distance, so that each leads near it.
- **Mesh**: a regular grid over the frame (64 cells along its longer side, grown by the
  resampling filter's reach for the edges' antialiasing) for the content outside, then each
  plane cut into 16 × 16 cells on top, exact along the planes' edges (the last triangle listed
  wins where they overlap, as for folds). The CPU compositor and the GPU renderer draw the same
  `PlacedMesh` as for a grid: nothing changed in the renderers. Tested against each other.
- **Editing**: Edit > Perspective Warp, on pixel layers, in the options bar Layout and Warp.
  Layout mode shows the layer unwarped: a drag draws a plane (a rectangle, its corners snapped
  to other planes' corners within 10 CSS pixels so that they share an edge); dragging a corner
  fits it to the image (its moved position follows by as much), and dropping it by another
  plane's corner joins them; a click selects a plane, Delete or Backspace removes it. Warp
  mode moves the corners, the layer warped live. A drag that would fold a plane (not convex)
  stays where it was. Enter or a double-click applies the planes' warp in either mode (one undo
  entry), Esc or undo cancels. A layer warped by planes opens in Warp mode, editable again.
  Removing every plane takes the warp away.
- **`.slop`**: node version 20 (schema 0.41), `params.warp` of kind `planes`.
- **Left for part 2**: Shift+click on an edge to straighten it (and keep it straight in later
  warps), the automatic Straighten buttons (horizontal, vertical, both), dragging an edge or a
  whole plane, and other layer kinds.
