# 0017 — Non-destructive transforms

Status: accepted (2026-09-30). Point 1 amended by [ADR 0038](0038-projective-transforms.md): the
layer's transform becomes projective, the affine maps a special case.

## Context

Moving, scaling and rotating layers, and resizing, cropping or rotating the whole image, are the
most frequent operations after opening a file. In most editors they resample pixels: every
change loses quality, and undoing is the only way back. SlopShop is non-destructive: the source
pixels must stay intact whatever the user does, and the result must stay fast at any size.

## Decision

1. **A transform per layer**: `Layer::transform`, a 2D affine map (scale, rotation, shear,
   translation; stored as six numbers) from the layer's content space (its raster's pixels, its
   mask's pixels) to its parent's space: the document for a top-level layer, the group's space
   for a layer in a group. A group's transform applies to everything inside it (composed at
   rendering). The identity by default. Masks are linked: they follow their layer's transform.
   Fill layers cover everything whatever their transform.
2. **Pixels never change.** Compositors sample each layer through the inverse of its composed
   transform. An integer translation is exact (a texel shift: no resampling, bit-exact export).
   Other transforms resample from the pyramid level matching the scale with a quality filter,
   identical on the CPU and the GPU (decided and tested in the step that allows them; until
   then, edits accept integer translations only).
3. **Image operations are transforms.** Image Size scales the top-level layers' transforms and
   the canvas; Canvas Size and Crop change the canvas and translate the layers; Rotate Image
   rotates them (by quarter turns exactly, or by any angle with the canvas grown to hold the
   turned one); Trim and Reveal All are crops computed from the image. One undoable batch each; pixels outside the canvas are kept, not cut.
4. **No render cache yet.** Sampling through a transform costs the same per frame as sampling
   without one; caches keyed by (node, region, level, revision) come with nodes that are costly
   to evaluate (filters, AI).
5. **Edits**: `SetLayerTransform`, and batch builders for "move these layers by (dx, dy)"
   (a group moves whole). **`.slop`**: `params.transform` on nodes whose transform is not the
   identity, written at node version 5 (older readers refuse them rather than misplace them).
6. **Interaction**: until the tools palette exists, a left drag on the image moves the selected
   layers (Photoshop's Move tool, V): live, one undo entry per drag, in whole document pixels;
   arrow keys move by 1 pixel, Shift+arrows by 10. Free Transform (Ctrl+T) follows with
   scaling and rotation.

## Alternatives

- **Resampling on each edit** (destructive, what most editors do): simple rendering, but quality
  degrades with every change and large images are rewritten.
- **Layer offsets only** (integer positions, like Photoshop's pixel layers): enough for moving,
  but scaling and rotation would still be destructive; an affine covers both with one field.
- **A cached transformed copy per layer**: faster to composite in theory, but memory and cache
  invalidation for no gain while sampling through the transform is cheap.

## Consequences

- Every place that maps document pixels to a raster's pixels goes through the layer's
  transform: compositors, tile planning, export defaults, covered-area tests.
- PSD layers can later be stored at their own size with a translation (instead of canvas-sized
  rasters), keeping their pixels outside the canvas.
- Thumbnails show a layer's content as is (not placed on the canvas).
