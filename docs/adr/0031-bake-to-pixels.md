# 0031 — Bake to Pixels: Rasterize, Merge, Merge Visible, Flatten

Status: accepted (2026-10-04, the maintainer's answers to the Layer menu audit: one submenu
for what is destructive, "Figer en pixels"; Rasterize keeps the transform; Flatten keeps
transparency; merging an adjustment layer bakes it into pixels like everything else).

## Context

SlopShop keeps everything editable: a layer's stack (ADR 0029), fill colors, groups,
transforms (ADR 0017). Photoshop's destructive layer commands (Rasterize, Merge Down, Merge
Layers, Merge Visible, Flatten Image) exist there partly because its model forces them; here
nothing forces them, but a user may still want plain pixels: one layer to hand over or to
paint on, fewer layers, an evaluated stack. Export already flattens without touching the
document.

## Decision

1. **One submenu, Layer > Bake to Pixels** ("Figer en pixels"), so that what loses
   editability is explicit and in one place: Rasterize, then Merge Layers (Ctrl+E; Merge Down
   with one layer), Merge Visible (Shift+Ctrl+E, Photoshop's; Export moves to
   Alt+Shift+Ctrl+W, Photoshop's Export As) and Flatten Image. Each is one undo entry.
2. **Rasterize** keeps each selected layer (its id, place, transform, opacity, blend mode,
   mask, clipping) and bakes its content: a pixel layer's stack becomes its original (and a
   painted mask's paint its mask); a fill becomes its color over the canvas; a group becomes
   its layers composited on their own (isolated). The transform is not baked: it costs
   nothing and stays editable. When the content starts before its space's origin (a fill moved
   right, a group's layers left of it), the content space moves by whole tiles and the mask
   grows alike (its tiles shared), so nothing moves on screen.
3. **Merge** composites the layers as they show, on their own (each with its mask, opacity,
   mode, and clipping only to a base merged too), into one pixel layer, normal and opaque, in
   the place of the topmost (Merge Down: of the lower one, named after it, refused onto a hidden
   layer as in Photoshop). Hidden layers merged are dropped. Pixels outside the canvas are
   kept. An adjustment layer merged down changes the pixels below it, baked like the rest.
   **Merge Visible** merges the visible layers of the top level, hidden ones staying; **Flatten
   Image** merges every layer, hidden ones dropped, keeping transparency (no background added).
4. **Where it runs**: `slopshop_core::bake` plans each composite as a scratch document whose
   whole canvas is the region to composite (shifted so that it starts at the origin), then
   builds the edit from the composited image. The app composites the scratch on the GPU (the
   CPU compositor otherwise), on a worker, in Copy Merged's format (half floats, working
   space, premultiplied).

## Alternatives

- **Merge as a dynamic composition** (a layer keeping its sources): that is a Smart Object by
  another name, and Group already keeps layers together without baking them.
- **Rasterize baking the transform**: resamples for nothing; the maintainer declined it.
- **Flatten onto white, as Photoshop**: Photoshop needs it for its Background layer, which
  SlopShop does not have.
- **Merging an adjustment down as an entry of the layer's stack**: non-destructive, but out
  of place in a menu meant for baking; declined.

## Consequences

- The merged layer blends normally at full opacity: layers merged from a mode other than
  Normal over what stays below change how they look, as in Photoshop.
- A pass-through group rasterized is composited isolated: an adjustment inside it no longer
  reaches below the group.
- Large merges cost a composite of their region (measured for New Layer from Visible, which
  shares the path: about 0.3 s for 12 MP, release build, RTX 4090 Laptop).
