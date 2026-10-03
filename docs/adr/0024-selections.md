# 0024 — Selections

Status: accepted (2026-10-01, maintainer's choices: a shared 16-bit mask, the outline computed
at the screen's resolution, not saved in the document).

## Context

Phase 3 begins with selections: what the next operation applies to (a mask, a crop, an
adjustment, later the brush and AI operations). Users expect Photoshop's tools (marquees,
lassos, AI selection) and its soft edges (anti-aliasing, feather). Documents can reach hundreds
of megapixels, so a selection must not cost memory or time in proportion to the canvas, and
its outline (the marching ants) must stay cheap to draw whatever its complexity.

## Decision

1. **A selection is a coverage mask**: a gray, **16-bit**, linear `RasterImage` the size of the
   canvas at the document origin (0 not selected, 65535 selected, values between for soft
   edges); nothing outside it is selected. The document holds `Option<Selection>` (`None`:
   nothing selected), immutable and shared like any raster, compared by identity. 16-bit
   rather than 8: a soft edge used as a mask for a strong correction does not band, and the
   cost is small (next point).
2. **Uniform tiles are shared**: tiles wholly selected or wholly unselected are one allocation
   each, and so are their pyramid tiles, so a selection costs memory in proportion to its
   outline. Combining with the current selection keeps the tiles it does not change.
3. **Shapes are rasterized exactly**: rectangles, ellipses (polygons within 1/100 pixel) and
   lasso polygons get each pixel's exact covered area (signed area accumulation, nonzero rule),
   or in/out by half coverage without anti-aliasing; one band of tile rows at a time over the
   shape's bounds, on every core. Feather is a Gaussian (three box blurs) of the tiles near an
   edge only, the canvas edge repeating, radius at most 250 px for now. Combining is per pixel:
   add = max, subtract = min(a, 1 − b), intersect = min.
4. **The selection is in the history**: `Edit::SetSelection` (select, deselect), undoable like
   any edit; its inverse keeps the previous mask by reference. Operations that change the canvas
   (crop, canvas or image size, rotation) deselect, as Photoshop does.
5. **Not saved in `.slop`**, as Photoshop does not save the active selection. Select > Save
   Selection stores it explicitly (amended below). Saving it now as a compatible addition
   would also break: an older SlopShop resaving the file keeps unknown fields but drops images
   no layer references.
6. **The marching ants are computed at the screen's resolution**: the engine follows the pixel
   edges where coverage crosses one half, on the pyramid level matching the zoom and only over
   the visible region, as polylines in document pixels (a budget of points: beyond it, a
   coarser level). The UI draws them as SVG with a CSS dash animation, so they cost the engine
   nothing between view changes and work with native presentation and with frames alike.

## Alternatives

- **8-bit masks** (Photoshop in 8-bit documents): half the memory on edge tiles, but soft edges
  can band under strong corrections.
- **Vector shapes rasterized on demand**: editable rectangles and lassos, but the magic wand,
  color ranges, AI selection and painted masks are pixels anyway: two systems.
- **Ants drawn by the GPU in the image**: no complexity limit, but the animation needs about ten
  renders a second, costly when frames travel over the IPC (macOS, Linux).

## Consequences

- `slopshop_core::selection`: shapes, combine modes, feather, inverse, bounds, outline;
  `Document::selection`, `Edit::SetSelection`.
- The app gets the selection tools (marquees, lassos) with their options, the Select menu,
  selection to layer mask, Image > Crop to the selection, and the ants overlay.
- AI selection (click, box, subject, text) produces the same masks; its inference runtime is a
  separate decision, under research.
- The Magic Wand compares colors as displayed (8-bit sRGB, straight alpha), on the composited
  document or a document holding only the sampled layer; it composites tile by tile, the tiles a
  contiguous fill reaches in parallel waves, with a bounded cache, so its memory does not grow
  with the canvas.
- Select > Modify: Expand, Contract and Border use the exact Euclidean distance to the outline
  (where coverage crosses one half), computed tile by tile near the outline only, so their
  corners are round and their cost does not grow with the canvas; a soft selection gets a crisp,
  anti-aliased edge at that distance. Smooth blurs and brings the edge back to one pixel;
  Feather blurs.
- Select > Grow and Similar are the Magic Wand started from every selected pixel at once (half
  covered or more), its tolerance taken around the range of their colors (each channel's lowest
  and highest), as in Photoshop: one engine for the three, the same sampling and color test.
- Select > Transform Selection reuses Free Transform's box (the UI) and the layers' resampling
  (ADR 0018, EWA from the matching pyramid level, exact for whole-pixel moves): soft edges stay
  soft, and a tile reading only uniform tiles of one value stays uniform.
- Quick Mask (Q) shows soft edges: a view overlay the GPU draws over the finished frame (after
  the display cache, in the encoded display space as Photoshop does), sampling the selection's
  pyramid like a layer's mask, at an opacity the user sets (half by default).
- The brush will respect the selection: it multiplies the stroke's coverage by the mask.
- Rasterizing touches every pixel of the shape's bounds once (0.65 s for an ellipse filling a
  600 MP canvas); interior tiles could be detected without being filled if that matters.

## Amendment (2026-10-04): saved selections

Maintainer's decisions of the Select menu audit: a saved selection is a **named object of the
document**, not an alpha channel shown as a channel. `Document::saved_selections` holds them
(stable `SavedSelectionId`s, never reused, in the order they were saved); inserting, removing,
renaming and replacing one are `Edit`s with exact inverses, so each is one undo entry. Canvas
operations (Crop, Canvas Size, Image Size, Image Rotation, Trim, Reveal All) transform them
with the image in the same undo entry, as Photoshop's channels follow the image: resampled like
a layer, exact for whole-pixel moves; one left with nothing keeps an empty mask. They are saved
in `.slop` (schema 0.16, `document.selections`): the maintainer allows breaking compatibility
until 1.0, so no compatibility flag protects them from older writers. Select > Save Selection
asks a name (an existing name replaces that saved selection), Select > Load Selection lists
them and replaces the selection; renaming, deleting and combining (add, subtract, intersect)
come with a Selections panel.
