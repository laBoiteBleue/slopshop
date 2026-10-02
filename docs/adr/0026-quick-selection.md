# 0026 — Quick Selection by color

Status: accepted (2026-10-02, maintainer's request: "Quick Selection must not be AI but work on
color, as in Photoshop").

## Context

Quick Selection (W) first used SAM 2.1 (ADR 0025): strokes became prompts and the model chose
an object. The maintainer wants Photoshop's tool: a brush whose selection grows to the similar
colors around the stroke and stops at the image's edges, without a model to download. The
documents are large; a stroke must answer in a fraction of a second.

## Decision

1. **A minimum cut on color models**, after Liu, Sun and Shum's Paint Selection (2009): a
   Gaussian mixture (5 components, k-means) of the stroke's colors and one of the other colors
   (Add: the unselected pixels; Subtract: the selection is "inside", the stroke "outside");
   neighbors (8) are linked by `50·exp(−β·Δ²)/length` (GrabCut's terms), so the cut follows
   edges. The stroke is pinned to its side, and in Add mode the selection too; only the region
   connected to the stroke is kept. Solved by Boykov and Kolmogorov's max-flow on the pixel grid
   (`slopshop_core::maxflow`, no dependency).
2. **Coarse to fine**: the cut runs first on the image reduced to 256 pixels on a side, then at
   full working resolution only in a band around that outline (Lombaert et al., 2005). Wide
   areas of similar colors (a sky) otherwise cost about a million augmentations (5 s on 1 MP;
   0.2 s with the band).
3. **On the colors as shown, at the view's resolution**: the region in view (the whole document
   when it fits), rendered by the GPU at most 1600 pixels on a side, as Object Selection does.
   The changed pixels become a selection through `select_scores` (bilinear, anti-aliased) and
   are combined at full resolution with the selection, which keeps its own edges; one working
   pixel of overlap on the selection's side avoids a seam.
4. **Photoshop's interaction**: each stroke is one undo entry; the first stroke of a new
   selection switches the options bar to Add; Alt subtracts; a click outside the image
   deselects. No Refine Edge option on this tool (Select > Refine Edges… stays available).
5. **Live** (maintainer's request): the selection grows while the stroke is painted. The
   stroke so far is sent as it grows (one request at a time, the latest waiting), cut on an
   800-pixel preview grid; its end on the 1600-pixel grid. The image and the selection before
   the stroke are prepared once per stroke; each result replaces the previous one within a
   gesture. Esc drops the stroke under way. The stroke is not drawn, only the brush circle.

## Alternatives

- **SAM 2.1** (before): selects whole objects, not color regions; a model to download.
- **Geodesic region growing** (seeds, color distance, a threshold): simpler, but leaks through
  weak edges and has no global trade-off.
- **A local window around the stroke** instead of the band: as fast, but straight cuts where a
  region continues past the window.

## Consequences

- `slopshop_core::quick_select` (models, cut, brush) and `slopshop_core::maxflow`; the
  `quick_select` command; `ai_segment` and the SAM Quick Selection session are removed.
- Detail finer than a working pixel (hair at a fitted view of a huge image) comes from zooming
  in (the region in view is used) or Select > Refine Edges….
