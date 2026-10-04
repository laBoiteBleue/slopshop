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
   (Amended 2026-10-04: with native presentation the GPU draws the ants in the frame instead,
   see the last amendment; this is the path of frames over IPC.)

## Alternatives

- **8-bit masks** (Photoshop in 8-bit documents): half the memory on edge tiles, but soft edges
  can band under strong corrections.
- **Vector shapes rasterized on demand**: editable rectangles and lassos, but the magic wand,
  color ranges, AI selection and painted masks are pixels anyway: two systems.
- **Ants drawn by the GPU in the image**: no complexity limit, but the animation needs about ten
  renders a second, costly when frames travel over the IPC (macOS, Linux). Chosen later for
  native presentation, where a present is cheap (amendment below); frames keep the SVG.

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
- Quick Mask (Q) is a state of the document, as Photoshop's channel (amended 2026-10-04 at the
  maintainer's request): `Document::quick_mask` holds the selection as a gray image the
  painting tools paint (`Edit::SetQuickMask`), the selection being an ordinary one meanwhile
  that limits them; entering and leaving are undoable edits (`Edit::enter_quick_mask`,
  `Edit::leave_quick_mask`), and canvas operations transform the mask with the image. It shows
  soft edges: a view overlay the GPU draws over the finished frame (after the display cache, in
  the encoded display space as Photoshop does), sampling the mask's pyramid like a layer's
  mask, at an opacity the user sets (half by default). Select and
  Mask's views use the same pass: the tint black or white wholly (On Black, On White), or the
  mask itself in gray.
- Select and Mask's Refine Edge Brush paints, with the Brush's engine, a coverage of where the
  matting model decides beside the outline's band (`plan_refinement_with`); edge detection
  always starts again from the selection the panel opened with, so strokes never compound.
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

## Amendment (2026-10-04): the marching ants drawn by the GPU

The ants lagged behind fast navigation on large images (a 233 MP image): the engine traced the
outline on the CPU, sent it over the IPC, and the page redrew an SVG path. Where the engine
presents to the window (Windows today, ADR 0002), the GPU now draws the ants in the frame,
from the selection's own coverage tiles, like Quick Mask's overlay (point 6 above still holds
for frames over IPC, see below):

- **A pass over the finished frame** (`ants_main` in `composite.wgsl`, `ViewOverlays::ants`),
  after the display cache's present, with the same bindings as Quick Mask's pass. The selection
  is sampled like a layer's mask at the view's pyramid level, so the cost is that of the frame's
  pixels, whatever the outline's complexity or the zoom, with no point budget.
- **The outline is the frame's pixels**: a pixel is on it when the selection covers it (half
  or more) and one of its four neighbours does not, one device pixel wide, inside the selection
  (the SVG's line ran on the pixel edges). Black or white by `((x + y + phase) / 4) & 1`:
  dashes of four, Photoshop-like. The canvas edge is not an outline (neighbours beyond it count
  as the pixel itself).
- **The phase is the clock's**: `Ants::at(time)` advances by one every 75 ms (8 pixels in 0.6 s,
  as the CSS animation did); the shell reads a shared clock at each present, so the dashes keep
  their place whatever the rate. The UI presents the view again every 66 ms, only while the ants
  are shown and moving (visible window, no reduced motion); a present is cheap with the display
  cache (ADR 0022: its tiles hit).
- **Placement**: the selection's plan takes an `Affine`: a drag's shift (whole pixels: an exact
  offset) and Select > Transform Selection's live matrix (resampled like a layer, ADR 0018) move
  the ants without touching the document.
- **Not drawn** over Quick Mask or Select and Mask's views (they show the selection
  themselves), nor without a selection; a Quick Mask on its own paints no ants even for a
  selection made meanwhile (the SVG still did).
- **Frames over IPC keep the SVG** (`SelectionOutline`, and `selection::outline` in the
  engine): macOS and Linux have no native presentation yet, and re-presenting ten times a
  second would cost a frame transfer each, which is the argument of the alternative below. The
  UI draws the SVG only when the engine does not draw the ants.

## Amendment (2026-10-04): the colors come from the GPU

Magic Wand, Grow, Similar and Color Range compared colors of a document composited on the CPU,
tile by tile. On documents of many layers that dominated (50 MP, 9 layers: 14 to 25 s). They now
read their pixels from a `PixelSource` (`slopshop_core::selection`): a function filling a region
with premultiplied working-space pixels, as `composite_region` does. `slopshop-core` cannot name
the renderer (core ← render), so the app passes `slopshop_render::export_renderer` (the export
path, ADR 0008) in; the tools' `*_from` functions take it, the older ones are the CPU
compositor's wrappers, which stays the fallback and the reference. A region the source cannot
render (a GPU error, too many layers for its tile cache) is composited on the CPU, so a failing
GPU never makes a selection wrong.

- The conversion to 8-bit display values and every test stay on the CPU, on the very code the
  CPU compositor's pixels went through: only the compositing moved. The contiguous fill and the
  coverage are unchanged.
- With a source, tiles are asked for a row of up to 32 at a time (2 MP, 32 MB of `f32`); three
  threads ask at once (a GPU call mostly waits on its upload and readback, measured: 3 threads
  render 50 MP in about half the time of one) while the others read the colors of the rows
  before; at most five rows are in memory, whatever the canvas.
- Rounding: the GPU and the CPU composite within float rounding (about 1e-4 relative), and a
  pixel whose value is at a 8-bit boundary may round the other way. Measured with the tests of
  `crates/slopshop-render/tests/selection.rs`: flat colors and exact blends select identically
  (0 pixels in 294,000, for every tool and tolerance tried, and for a 50 MP document of 2 or 9
  layers); a document of resampled wide-gamut gradients differs in at most a pixel of 294,000
  per selection (6 displayed colors of 294,000, by one level). The colors a Color Range compares
  with are sampled from the same source, so a sample is never one level off its own pixel.
- Cost: a GPU call has a latency (about 3 ms for a tile, 40 ms for a 2 MP row, readback of
  `f32` pixels bound), so the gain depends on what the CPU spends per pixel. On an RTX 4090
  Laptop, 50 MP: with 2 layers, tools that read every tile (non contiguous, Color Range) take
  half the time (0.8 s to 0.4 s); a contiguous fill over a large region is break-even (0.7 s
  both: waves of a few tiles pay the call latency); with 9 layers (blend modes, a resampled one)
  all of them are 5 to 11 times faster (14 to 25 s down to 2 to 3.5 s). Reading 8-bit values on
  the GPU rather than `f32` (a quarter of the readback) would cut the rest, as would a
  speculative prefetch of the contiguous fill's next tiles: not done.
