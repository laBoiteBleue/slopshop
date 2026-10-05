# 0037 — Liquify: a displacement field kept as an entry of the layer's stack

Status: accepted (2026-10-05, from the maintainer's decisions on the Filter menu, ADR 0034
point 10).

## Context

Liquify pushes, turns, pinches and smooths the pixels of a layer with a brush. In Photoshop it
is destructive (or a Smart Filter whose mesh is saved). ADR 0034 decided it is a stack entry
holding what the brush did, edited again in its own workspace, with an eye and one undo entry
(never a filter layer). The maintainer works on layers of 233 megapixels: the field must not
take memory in proportion to the layer, strokes must feel immediate, and a layer being edited
must show at once at any zoom.

## Decision

1. **The entry holds a displacement field**, backward mapping in the layer's own pixels:
   `output(x) = input(x + d(x))`, `input` the result of the entries below it, sampled
   bilinearly in premultiplied values of the document's blend space (the layer's edges repeating
   outward), the result rounded to the layer's format. The blend space is kept with the entry.
2. **The field is a grid coarser than the layer, tiled and sparse.** One node (two `f32`) per 1,
   2 or 4 pixels a side, chosen from the layer's size at creation (up to 4 MP, up to 32 MP,
   beyond), interpolated bilinearly between nodes. A dense field is then never above about
   120 MB (a 233 MP layer: 14.6 million nodes) and an edit holds a fraction of that (31 MB
   after the timing run's strokes of up to 3000 pixels on 24 MP, 10 MB on 233 MP). Its tiles are aligned with the
   layer's own (the same tile coordinates, `256 / cell` nodes a side): a tile no stroke touched
   does not exist, evaluating a layer shares the input's tiles wherever the field's tile and its
   eight neighbours are absent, a layer grown by whole tiles grows its field, and a tile is
   small enough to copy on write (512 KB with a node a pixel, 32 KB at the coarsest cell). Nodes are `f32`: many small strokes pile up
   without quantizing.
3. **The freeze mask is the same grid** (8 bits a node): frozen nodes are not moved by any
   tool, a partly frozen one partly.
4. **Strokes compose exactly.** A dab shifts the pixels shown by `s(x)`; the field becomes
   `d'(x) = s(x) + d(x + s(x))` (the old field read where the pixels now come from), so that the
   field stays what the strokes did however many pile up. Forward Warp and Push Left follow
   the brush's motion, Twirl, Pucker and Bloat turn or pinch around its center, Reconstruct
   brings the field toward nothing, Smooth averages it, Freeze and Thaw edit the mask; Photoshop's
   brush settings (Size, Density, Pressure, Rate) set the falloff, the strength of what follows
   the pointer and the speed of what acts while it is held still. Dabs are placed every 6 % of the
   brush's size; a dab of a large brush runs on every core.
5. **Evaluation.** The whole layer (export, tools) is warped tile by tile, reading the input's
   tiles where the field says, with no margin to copy. Looks at the part shown at a pyramid
   level take the input's tiles around it with a margin of the field's reach (its largest
   displacement, kept with the entry) and warp only the part shown, as a filter's look does
   (ADR 0034, 0035); the quick look of the whole layer is the same job at the coarsest level of
   at most 4 MP. The entry is a materialization point like a filter: it caches what it was
   applied to and its result, and the entries above evaluate from it. **CPU first**: the GPU
   look declines these jobs; a GPU warp (sampling the field as a texture) is a follow-up.
6. **The workspace** is a modal window of its own: the engine holds a session (the field being
   edited, the layer's pixels below the entry, a history of the field's states, one per stroke:
   cloning a field shares its tiles), the UI sends strokes in pieces and asks for frames of the
   layer seen through the field at the zoom it shows (the pyramid level the zoom asks for,
   8-bit sRGB RGBA as raw binary, the frozen area tinted red). The document does not change
   until OK, which makes the field one entry (`Edit::apply_liquify`, or `Edit::set_liquify` for
   an entry edited again; a field with nothing left displaced removes the entry). Navigation is
   the app's: the wheel zooms about the pointer, the middle button (or Space) pans; there is
   no Hand nor Zoom tool.
7. **`.slop` 0.23**: an entry `{"liquify": {"displacement", "frozen", "cell", "space"}}` whose
   two images are the field and the mask at one pixel per node (a gray + alpha `f32` image whose
   channels are `dx` and `dy`, never converted, and a gray 8-bit one), so that the file's blob
   store hashes, deduplicates and compresses them as any image, a tile no stroke touched being
   one shared zero tile. Older readers refuse the entry as coming from a newer SlopShop.
8. **Not in this version**: Face-Aware Liquify, saved meshes, the mesh display, pen pressure, the
   selection as a limit (Photoshop's Liquify ignores it as well), Alt+Push Left to the right,
   Thaw All and Freeze All.

## Alternatives

- **A filter layer**: ADR 0034 declined it; its multi-pass compositor does not exist.
- **A dense full-resolution field**: 1.9 GB at 233 MP before any stroke; a field coarser than
  the layer interpolates well under brushes of tens of pixels or more.
- **A mesh of control points (Photoshop's)**: resolution-independent and small, but strokes of
  every tool compose approximately and Reconstruct and Smooth have no clean meaning on a mesh;
  a field of the layer's grid is exact and local.
- **Replaying the strokes as the entry's content**: smallest in the file, but evaluation
  grows with the history and a brush's falloff must stay bit-exact across versions.
- **Compositing the displacement as `d' = d + s`**: simpler, but wrong where the old field
  already moved the pixels the dab reads, and strokes over a twirl or a push distort.
- **GPU warp first**: the CPU measured fast enough (below); the GPU needs the field and the
  input as textures per tile and a second path next to ADR 0035's, for a gain nobody waited for.

## Consequences

- `slopshop-core` gains `liquify` (the field, the tools, the warp) and a stack entry kind that
  filters share the cache machinery of (`last_filter`, looks, `flat`); a look job may carry a
  warp rather than filter steps. The Tauri shell gains a session and seven commands, the app
  a workspace and its loop (`StrokeQueue`, `FrameLoop`).
- Editing an entry that is not the topmost one evaluates what is below it for the workspace; what
  is above it follows when it is accepted, as for any entry (ADR 0034).
- Measured on a 13th Gen Intel Core i9-13980HX (32 threads) with an NVIDIA GeForce RTX 4090
  Laptop GPU, which this code does not use, in release: a Forward Warp move takes 0.04 ms with a
  brush of 50 pixels, 1 ms at 300, 4 ms at 1000 and 16 ms at 3000 on 24 MP (6 ms at 3000 on
  233 MP); a frame of the workspace (1920 × 1080) takes 22 ms at 100 % and 13 to 17 ms zoomed
  out, 3840 × 2160 at 50 % 43 to 63 ms; the whole layer with 31 MB of field evaluated in 0.22 s
  (24 MP) and 1.1 s (233 MP, a long stroke across a part of it).
- Edges: a field that pushes the layer's pixels away shows its edge repeated, not transparent.
- The GPU warp, a limit by the selection and the opposite tools are open.
